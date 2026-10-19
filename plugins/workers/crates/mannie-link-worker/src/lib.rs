//! The worker's side of the link (ADR-0016, ADR-0028; plugins/workers/docs/
//! pairing-and-link.md): the server, whose connections reach only pairing
//! until their key is the pinned master's, the services over the runtime's
//! assigned tasks, the lease vault, and the pin of the master in the data
//! folder.

mod leases;
mod pairing;
mod services;

use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

use mannie_host_kit::runtime::Runtime;
use mannie_plugin_sdk::contracts::Clock;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Server;
use tonic::{Request, Status};

pub use leases::LeaseVault;
pub use pairing::{CODE_FILE, write_code};

use mannie_link::codec::MAX_MESSAGE_BYTES;
use mannie_link::host::WorkerDetails;
use mannie_link::keys::{KeyError, KeyPin, LinkKey};
use mannie_link::proto::approvals::approvals_service_server::ApprovalsServiceServer;
use mannie_link::proto::events::events_service_server::EventsServiceServer;
use mannie_link::proto::leases::leases_service_server::LeasesServiceServer;
use mannie_link::proto::pairing::pairing_service_server::PairingServiceServer;
use mannie_link::proto::session::session_service_server::SessionServiceServer;
use mannie_link::proto::tasks::tasks_service_server::TasksServiceServer;
use mannie_link::tls::{self, Peer};

/// The file name of the master's pin in the data folder.
pub const PIN_FILE: &str = "master-pin";

/// The file name of the worker's private key in the data folder.
pub const KEY_FILE: &str = "link-key";

/// Why the worker's side of the link failed.
#[derive(Debug, thiserror::Error)]
pub enum WorkerLinkError {
    /// The TLS configuration could not be built.
    #[error("the link's TLS could not be set up: {0}")]
    Tls(#[from] rustls::Error),
    /// The server failed.
    #[error("the link's server failed: {0}")]
    Server(#[from] tonic::transport::Error),
    /// A file of the data folder could not be read or written.
    #[error("a file of the link could not be read or written: {0}")]
    Io(#[from] io::Error),
    /// The worker's key could not be made, or the stored one is damaged.
    #[error("the worker's key of the link could not be made or read: {0}")]
    Key(#[from] KeyError),
}

/// The worker's key from `<data>/link-key`, made and kept there on first
/// use. Only the worker's account can read the file.
pub fn load_or_create_key(data: &Path) -> Result<LinkKey, WorkerLinkError> {
    let path = data.join(KEY_FILE);
    match std::fs::read(&path) {
        Ok(pkcs8) => Ok(LinkKey::from_pkcs8(pkcs8)?),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let key = LinkKey::generate()?;
            write_private(&path, key.pkcs8())?;
            tracing::info!(pin = key.pin().short(), "made the worker's key of the link");
            Ok(key)
        }
        Err(error) => Err(error.into()),
    }
}

/// Writes a file that only the worker's account reads, replacing it at
/// once: the new content goes to a temporary file first, which is renamed.
pub(crate) fn write_private(path: &Path, content: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("new");
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        io::Write::write_all(&mut file, content)?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, path)
}

/// The master that the worker is paired with: its pin, in memory and in
/// `<data>/master-pin`.
#[derive(Debug)]
pub struct Binding {
    path: PathBuf,
    pin: RwLock<Option<KeyPin>>,
}

impl Binding {
    /// The binding that the data folder holds, if any.
    pub fn load(data: &Path) -> io::Result<Self> {
        let path = data.join(PIN_FILE);
        let pin = match std::fs::read_to_string(&path) {
            Ok(text) => KeyPin::parse(&text),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        Ok(Self {
            path,
            pin: RwLock::new(pin),
        })
    }

    /// The pin of the master, while the worker is paired.
    pub fn pin(&self) -> Option<KeyPin> {
        self.pin
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Pins a master.
    pub(crate) fn set(&self, pin: &KeyPin) -> io::Result<()> {
        write_private(&self.path, pin.as_str().as_bytes())?;
        *self.pin.write().unwrap_or_else(PoisonError::into_inner) = Some(pin.clone());
        Ok(())
    }

    /// Forgets the master.
    pub fn clear(&self) -> io::Result<()> {
        *self.pin.write().unwrap_or_else(PoisonError::into_inner) = None;
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

/// What the worker's side of the link serves.
#[derive(Debug)]
pub struct WorkerLink {
    /// The worker's runtime.
    pub runtime: Runtime,
    /// The worker's key.
    pub key: LinkKey,
    /// The worker's name, from its configuration.
    pub name: String,
    /// What the worker says about itself in `Hello`.
    pub details: WorkerDetails,
    /// The worker's version of Mannie.
    pub version: String,
    /// The data folder, which holds the master's pin and the pairing code.
    pub data: PathBuf,
    /// The leases of the worker's tasks, which unpairing drops.
    pub leases: Arc<LeaseVault>,
    /// The clock of the pairing code's lifetime.
    pub clock: Arc<dyn Clock>,
}

/// Lets only the pinned master past the interceptor; every other key reaches
/// only pairing.
fn paired_only(
    binding: Arc<Binding>,
) -> impl FnMut(Request<()>) -> Result<Request<()>, Status> + Clone {
    move |request: Request<()>| {
        let peer = request.extensions().get::<Peer>();
        let pinned = binding.pin();
        match (peer, pinned) {
            (Some(peer), Some(pinned)) if peer.pin == pinned => Ok(request),
            (Some(peer), _) => {
                tracing::debug!(
                    peer = peer.pin.short(),
                    "a key that is not the pinned master's called past pairing"
                );
                Err(Status::permission_denied(
                    "this worker is not paired with this key",
                ))
            }
            (None, _) => Err(Status::permission_denied("the connection has no peer")),
        }
    }
}

/// Serves the link on `listener` until `shutdown` resolves. Then it takes no
/// new calls, lets the calls that run finish, and ends the streams that a
/// master keeps open: its watch of the tasks and its subscriptions, which
/// end only with the runtime and would otherwise hold the stop open.
pub async fn serve(
    link: WorkerLink,
    listener: TcpListener,
    shutdown: impl Future<Output = ()> + Send,
) -> Result<(), WorkerLinkError> {
    let config = tls::server_config(&link.key)?;
    let binding = Arc::new(Binding::load(&link.data)?);
    let own_pin = link.key.pin();
    tracing::info!(
        pin = own_pin.short(),
        paired = binding.pin().is_some(),
        "the worker's link listens"
    );
    let pairing = pairing::Pairing::new(
        link.data.clone(),
        link.name.clone(),
        own_pin,
        Arc::clone(&binding),
        Arc::clone(&link.clock),
    );
    let (stopping, stops) = watch::channel(false);
    let services = services::Services::new(link, Arc::clone(&binding), stops);
    let guard = paired_only(Arc::clone(&binding));
    let incoming = tls::incoming(listener, config);
    Server::builder()
        .add_service(
            PairingServiceServer::new(pairing)
                .max_decoding_message_size(MAX_MESSAGE_BYTES)
                .max_encoding_message_size(MAX_MESSAGE_BYTES),
        )
        .add_service(InterceptedService::new(
            SessionServiceServer::new(services.clone())
                .max_decoding_message_size(MAX_MESSAGE_BYTES)
                .max_encoding_message_size(MAX_MESSAGE_BYTES),
            guard.clone(),
        ))
        .add_service(InterceptedService::new(
            TasksServiceServer::new(services.clone())
                .max_decoding_message_size(MAX_MESSAGE_BYTES)
                .max_encoding_message_size(MAX_MESSAGE_BYTES),
            guard.clone(),
        ))
        .add_service(InterceptedService::new(
            EventsServiceServer::new(services.clone())
                .max_decoding_message_size(MAX_MESSAGE_BYTES)
                .max_encoding_message_size(MAX_MESSAGE_BYTES),
            guard.clone(),
        ))
        .add_service(InterceptedService::new(
            ApprovalsServiceServer::new(services.clone())
                .max_decoding_message_size(MAX_MESSAGE_BYTES)
                .max_encoding_message_size(MAX_MESSAGE_BYTES),
            guard.clone(),
        ))
        .add_service(InterceptedService::new(
            LeasesServiceServer::new(services)
                .max_decoding_message_size(MAX_MESSAGE_BYTES)
                .max_encoding_message_size(MAX_MESSAGE_BYTES),
            guard,
        ))
        .serve_with_incoming_shutdown(incoming, async move {
            shutdown.await;
            stopping.send_replace(true);
        })
        .await?;
    Ok(())
}
