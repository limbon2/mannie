//! The worker's side of the test tools: a worker's runtime served over the
//! link on loopback, a relay that can cut every connection, lose the
//! worker's answers or refuse new connections, and a relay in the middle with
//! a key of its own (plugins/workers/docs/pairing-and-link.md, acceptance).
//! The master's side of the tests is [`crate::master`].

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mannie_host_kit::logging::Redactor;
use mannie_plugin_harness::runtime::{WorkerHarness, WorkerOptions};
use mannie_plugin_harness::types::SystemClock;
use mannie_plugin_sdk::contracts::Clock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;

use mannie_link::host::WorkerDetails;
use mannie_link::keys::LinkKey;
use mannie_link::{levels, tls};
use mannie_link_worker::{self as worker, LeaseVault, WorkerLink};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How a linked worker is set up.
#[derive(Debug, Clone)]
pub struct LinkedWorkerOptions {
    /// The options of the worker's runtime.
    pub worker: WorkerOptions,
    /// The version that the worker says it runs.
    pub version: String,
}

impl Default for LinkedWorkerOptions {
    fn default() -> Self {
        Self {
            worker: WorkerOptions::default(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

/// A worker's runtime served over the link on a loopback port.
#[derive(Debug)]
pub struct LinkedWorker {
    /// The worker's runtime and its model.
    pub harness: WorkerHarness,
    /// The address the worker listens on.
    pub address: String,
    /// The worker's leases.
    pub leases: Arc<LeaseVault>,
    /// The redactor that hides the leases.
    pub redactor: Arc<Redactor>,
    /// The worker's key.
    pub key: Arc<LinkKey>,
    shutdown: Option<oneshot::Sender<()>>,
    server: Option<JoinHandle<()>>,
}

impl LinkedWorker {
    /// A worker with the default options.
    pub async fn start() -> Self {
        Self::with_options(LinkedWorkerOptions::default()).await
    }

    /// A worker with `options`, served at once.
    ///
    /// # Panics
    ///
    /// When the worker cannot be set up, which fails the test.
    pub async fn with_options(options: LinkedWorkerOptions) -> Self {
        let redactor = Arc::new(Redactor::new());
        let leases = Arc::new(LeaseVault::new(Arc::clone(&redactor)));
        let vault: Arc<dyn mannie_plugin_harness::runtime::CredentialVault> =
            Arc::<LeaseVault>::clone(&leases);
        let harness = WorkerHarness::with_options(WorkerOptions {
            vault: Some(vault),
            ..options.worker
        })
        .await;
        let key = Arc::new(LinkKey::generate().expect("a key"));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port");
        let address = listener.local_addr().expect("an address").to_string();
        let link = WorkerLink {
            runtime: harness.runtime.clone(),
            key: LinkKey::from_pkcs8(key.pkcs8().to_vec()).expect("the same key"),
            name: harness.host_name.clone(),
            details: WorkerDetails {
                purpose: "Builds and tests the backend".to_owned(),
                facts: None,
                ceiling: None,
                version: Some(options.version.clone()),
                max_parallel_tasks: 4,
            },
            version: options.version,
            data: harness.data_folder(),
            leases: Arc::clone(&leases),
            clock: Arc::new(SystemClock),
        };
        let (shutdown, stopped) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let stopping = async {
                let _stopped = stopped.await;
            };
            if let Err(error) = worker::serve(link, listener, stopping).await {
                tracing::error!(%error, "the test worker's link failed");
            }
        });
        Self {
            harness,
            address,
            leases,
            redactor,
            key,
            shutdown: Some(shutdown),
            server: Some(server),
        }
    }

    /// The data folder, which holds the master's pin and the pairing code.
    pub fn data(&self) -> PathBuf {
        self.harness.data_folder()
    }

    /// A new pairing code, as `mannie-worker pair` writes it.
    ///
    /// # Panics
    ///
    /// When the code cannot be written, which fails the test.
    pub fn new_code(&self) -> String {
        worker::write_code(&self.data(), SystemClock.now())
            .expect("the code is written")
            .display()
    }

    /// Stops serving the link.
    pub async fn stop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _sent = shutdown.send(());
        }
        if let Some(server) = self.server.take() {
            let _ended = server.await;
        }
    }
}

/// A relay between the master and a worker: it copies the bytes of each
/// connection, can cut every connection, can refuse new ones while blocked,
/// can lose what the worker sends, and can send new connections to another
/// address.
#[derive(Debug)]
pub struct Relay {
    /// The address the master connects to.
    pub address: String,
    /// Cuts the current connections when it changes.
    cut: watch::Sender<u64>,
    blocked: Arc<Mutex<bool>>,
    answers_lost: Arc<AtomicBool>,
    target: Arc<Mutex<String>>,
    task: JoinHandle<()>,
}

/// Copies one connection both ways until either side ends it. While
/// `answers_lost` is set, what the worker sends is read and dropped, as when
/// a link goes down after a call reached the worker and before its answer
/// reached the master.
async fn relay_connection(inbound: TcpStream, outbound: TcpStream, answers_lost: Arc<AtomicBool>) {
    let (mut from_master, mut to_master) = inbound.into_split();
    let (mut from_worker, mut to_worker) = outbound.into_split();
    let requests = tokio::io::copy(&mut from_master, &mut to_worker);
    let answers = async {
        let mut buffer = vec![0_u8; 16 * 1024];
        loop {
            let read = from_worker.read(&mut buffer).await?;
            if read == 0 {
                return Ok::<(), std::io::Error>(());
            }
            if !answers_lost.load(Ordering::SeqCst) {
                to_master.write_all(&buffer[..read]).await?;
            }
        }
    };
    tokio::select! {
        _copied = requests => {}
        _copied = answers => {}
    }
}

impl Relay {
    /// A relay to `target`.
    ///
    /// # Panics
    ///
    /// When it cannot listen, which fails the test.
    pub async fn start(target: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port");
        let address = listener.local_addr().expect("an address").to_string();
        let cut = watch::Sender::new(0);
        let blocked = Arc::new(Mutex::new(false));
        let answers_lost = Arc::new(AtomicBool::new(false));
        let target = Arc::new(Mutex::new(target.to_owned()));
        let task = {
            let cut = cut.clone();
            let blocked = Arc::clone(&blocked);
            let answers_lost = Arc::clone(&answers_lost);
            let target = Arc::clone(&target);
            tokio::spawn(async move {
                while let Ok((inbound, _address)) = listener.accept().await {
                    if *lock(&blocked) {
                        drop(inbound);
                        continue;
                    }
                    let target = lock(&target).clone();
                    let mut cut = cut.subscribe();
                    let answers_lost = Arc::clone(&answers_lost);
                    tokio::spawn(async move {
                        let Ok(outbound) = TcpStream::connect(&target).await else {
                            return;
                        };
                        tokio::select! {
                            () = relay_connection(inbound, outbound, answers_lost) => {}
                            _cut = cut.changed() => {}
                        }
                    });
                }
            })
        };
        Self {
            address,
            cut,
            blocked,
            answers_lost,
            target,
            task,
        }
    }

    /// Loses what the worker sends on every connection from now on, or
    /// stops losing it.
    pub fn set_answers_lost(&self, lost: bool) {
        self.answers_lost.store(lost, Ordering::SeqCst);
    }

    /// Ends every connection and lets new ones through as they come, so the
    /// master connects again: the link drops and comes back.
    pub fn drop_link(&self) {
        self.set_answers_lost(false);
        self.cut();
    }

    /// Sends new connections to `target`.
    pub fn set_target(&self, target: &str) {
        target.clone_into(&mut lock(&self.target));
    }

    /// Cuts every current connection.
    pub fn cut(&self) {
        self.cut.send_modify(|count| *count += 1);
    }

    /// Refuses new connections, or lets them through again.
    pub fn set_blocked(&self, blocked: bool) {
        *lock(&self.blocked) = blocked;
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// A relay in the middle with a key of its own: it ends the master's TLS
/// with its key and opens its own TLS to the worker with the same key, then
/// copies the plaintext both ways, as an attacker in the middle of a pairing
/// would.
#[derive(Debug)]
pub struct KeyInTheMiddle {
    /// The address the master connects to.
    pub address: String,
    task: JoinHandle<()>,
}

impl KeyInTheMiddle {
    /// A relay to `target` with a new key.
    ///
    /// # Panics
    ///
    /// When it cannot listen, which fails the test.
    pub async fn start(target: &str) -> Self {
        let key = LinkKey::generate().expect("a key");
        let server = tls::server_config(&key).expect("the relay's TLS");
        let client = tls::client_config(&key, None).expect("the relay's TLS");
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback port");
        let address = listener.local_addr().expect("an address").to_string();
        let target = target.to_owned();
        let task = tokio::spawn(async move {
            let acceptor = tokio_rustls::TlsAcceptor::from(server);
            while let Ok((inbound, _address)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let client = Arc::clone(&client);
                let target = target.clone();
                tokio::spawn(async move {
                    let Ok(mut inbound) = acceptor.accept(inbound).await else {
                        return;
                    };
                    let Ok((mut outbound, _pin)) = tls::connect(&target, client).await else {
                        return;
                    };
                    let _copied = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                });
            }
        });
        Self { address, task }
    }
}

impl Drop for KeyInTheMiddle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// This version's levels, for tests that simulate another version.
pub fn current_levels() -> std::collections::HashMap<String, u32> {
    levels::levels()
}
