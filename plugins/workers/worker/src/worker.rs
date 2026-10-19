//! Starting and stopping the worker (ADR-0015, ADR-0028): logging with the
//! redactor first, then the hardening, the store, the agent runtime for this
//! host under its ceiling, and the link. The lease vault shares the log's
//! redactor, so a leased credential never reaches a log line.

use std::fs;
use std::future::Future;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use mannie_host_kit::logging::{ErrorChain, LogConfig, LoggingError, LoggingGuard, Redactor};
use mannie_host_kit::provider::{HttpClient, ProviderRegistry};
use mannie_host_kit::runtime::{
    CredentialVault, MachineSettings, MentionRegistry, Runtime, RuntimeConfig, RuntimeDeps,
    ToolsDeps,
};
use mannie_host_kit::store::{Store, StoreError, StoreOptions};
use mannie_host_kit::tools::{MutationLock, SystemFacts, ToolRegistry};
use mannie_host_kit::types::SystemClock;
use mannie_link::host::{CeilingView, WorkerDetails};
use mannie_link_worker::{LeaseVault, WorkerLink, WorkerLinkError};
use mannie_plugin_sdk::contracts::Clock;

use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::agent::{self, Host, WORKER_AGENT};
use crate::config::Config;
use crate::paths::Folders;
use crate::{health, host};

/// The file name of the worker's database in its data folder.
pub(crate) const DATABASE_FILE: &str = "mannie-worker.sqlite";

/// The start of the worker's log file names.
const LOG_FILE_PREFIX: &str = "mannie-worker";

/// The worker's version, which `Hello` and the health marker report.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// How long a stop waits for the link's running calls to finish before it
/// closes the link's connections.
const LINK_STOP_GRACE: Duration = Duration::from_secs(10);

/// What the worker needs to start.
#[derive(Debug, Clone)]
pub(crate) struct WorkerOptions {
    /// The checked configuration.
    pub(crate) config: Config,
    /// The system's folders, whose configuration and program folders the
    /// tools protect.
    pub(crate) folders: Folders,
    /// Whether log lines also go to standard error, for a worker in a terminal.
    pub(crate) console_logging: bool,
}

/// Why the worker could not start.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StartError {
    /// The data folder could not be created or made private.
    #[error("could not prepare the data folder {path}")]
    DataDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Logging could not start.
    #[error("could not start logging")]
    Logging(#[from] LoggingError),
    /// The store did not open, for example because a newer version of Mannie wrote it.
    #[error("could not open the database")]
    Store(#[from] StoreError),
    /// The HTTP client of the provider adapters could not be set up.
    #[error("could not set up the HTTP client for model providers")]
    HttpClient(#[source] mannie_host_kit::provider::TransportError),
    /// The runtime could not recover the tasks of the last run.
    #[error("could not recover the tasks of the last run")]
    Runtime(#[source] mannie_host_kit::store::RuntimeStoreError),
    /// The worker's key could not be read or made.
    #[error("could not read or make the worker's key")]
    Key(#[source] WorkerLinkError),
    /// The worker could not listen on its address.
    #[error(
        "could not listen on {address}; another program may use the port, or the address is not this host's"
    )]
    Listen {
        address: SocketAddr,
        #[source]
        source: std::io::Error,
    },
}

/// A running worker.
#[derive(Debug)]
pub(crate) struct Worker {
    runtime: Runtime,
    store: Store,
    link: JoinHandle<Result<(), WorkerLinkError>>,
    stop_link: oneshot::Sender<()>,
    data_directory: PathBuf,
    logging: LoggingGuard,
}

/// What the worker says about its ceiling in `Hello`.
fn ceiling_view(config: &Config) -> CeilingView {
    let ceiling = &config.ceiling;
    CeilingView {
        mode: ceiling.mode,
        allowed_folders: ceiling
            .allowed_folders
            .iter()
            .map(|folder| folder.display().to_string())
            .collect(),
        rules: ceiling
            .rules
            .iter()
            .map(|rule| match &rule.folder {
                Some(folder) => format!("{} (in {folder})", rule.words.join(" ")),
                None => rule.words.join(" "),
            })
            .collect(),
        accepts_master_full_access: config.accepts_master_full_access,
    }
}

/// A folder of the worker's own, in the canonical form that the policy
/// compares, or as it is named when it does not exist.
fn protected_folder(folder: &Path) -> PathBuf {
    match mannie_host_kit::tools::canonical_folder(folder) {
        Ok(canonical) => PathBuf::from(canonical),
        Err(_missing) => folder.to_path_buf(),
    }
}

impl Worker {
    /// Prepares the data folder, starts logging, applies the hardening,
    /// opens the store, starts the runtime and serves the link.
    pub(crate) async fn start(options: &WorkerOptions) -> Result<Self, StartError> {
        let config = &options.config;
        let data_directory = &config.data_dir;
        create_private_directory(data_directory).map_err(|source| StartError::DataDirectory {
            path: data_directory.clone(),
            source,
        })?;
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let redactor = Redactor::new();
        let log_config = LogConfig::new(
            data_directory.join("logs"),
            LOG_FILE_PREFIX,
            options.console_logging,
        );
        let logging = mannie_host_kit::logging::init(&log_config, &redactor, Arc::clone(&clock))?;
        crate::hardening::apply();
        tracing::info!(
            version = VERSION,
            data_directory = %data_directory.display(),
            ceiling = config.ceiling.mode.as_str(),
            "the worker is starting"
        );
        let database = data_directory.join(DATABASE_FILE);
        let store_options = StoreOptions::new(data_directory.join("backups"), Arc::clone(&clock));
        let store = Store::open(&database, &store_options).inspect_err(|error| {
            tracing::error!(error = %ErrorChain(error), error.code = error.code(), "could not open the database");
        })?;
        let leases = Arc::new(LeaseVault::new(Arc::new(redactor)));
        let system = SystemFacts::detect();
        let facts = host::host_facts(&system);
        let runtime =
            start_runtime(options, store.clone(), &leases, system, Arc::clone(&clock)).await?;
        let key =
            mannie_link_worker::load_or_create_key(data_directory).map_err(StartError::Key)?;
        let listener =
            TcpListener::bind(config.listen)
                .await
                .map_err(|source| StartError::Listen {
                    address: config.listen,
                    source,
                })?;
        let link = WorkerLink {
            runtime: runtime.clone(),
            key,
            name: config.name.clone(),
            details: WorkerDetails {
                purpose: config.purpose.clone(),
                facts: Some(facts),
                ceiling: Some(ceiling_view(config)),
                version: Some(VERSION.to_owned()),
                max_parallel_tasks: config.max_parallel_tasks,
            },
            version: VERSION.to_owned(),
            data: data_directory.clone(),
            leases,
            clock,
        };
        let (stop_link, stopped) = oneshot::channel::<()>();
        let link = tokio::spawn(mannie_link_worker::serve(link, listener, async {
            let _stopped = stopped.await;
        }));
        if let Err(error) = health::write(data_directory) {
            tracing::error!(%error, "could not write the health marker; `mannie-worker update` cannot confirm this program");
        }
        tracing::info!(
            database = %store.path().display(),
            modules = ?store.modules(),
            listen = %config.listen,
            "the worker started"
        );
        Ok(Self {
            runtime,
            store,
            link,
            stop_link,
            data_directory: data_directory.clone(),
            logging,
        })
    }

    /// Runs until `stop` resolves, or until the link fails, which is an
    /// error that the service manager answers with a restart.
    pub(crate) async fn run_until(mut self, stop: impl Future<Output = ()>) -> anyhow::Result<()> {
        let failed = tokio::select! {
            () = stop => None,
            ended = &mut self.link => Some(ended),
        };
        let outcome = match failed {
            None => Ok(()),
            Some(Ok(Ok(()))) => Err(anyhow::anyhow!("the link stopped serving")),
            Some(Ok(Err(error))) => Err(anyhow::Error::new(error).context("the link failed")),
            Some(Err(error)) => {
                Err(anyhow::Error::new(error).context("the link ended unexpectedly"))
            }
        };
        if let Err(error) = &outcome {
            tracing::error!(
                error = format!("{error:#}"),
                "the worker stops because its link failed"
            );
        }
        self.stop().await;
        outcome
    }

    /// Stops the link, lets the runtime end its runs, closes the store and
    /// writes out the remaining log lines.
    async fn stop(mut self) {
        tracing::info!("the worker is stopping");
        if let Err(error) = health::remove(&self.data_directory) {
            tracing::warn!(%error, "could not remove the health marker");
        }
        let _sent = self.stop_link.send(());
        // A master that keeps a call open must not keep the service manager
        // waiting: the calls that change something run to their end on tasks
        // of their own, so closing the connections loses only answers.
        if !self.link.is_finished() {
            match tokio::time::timeout(LINK_STOP_GRACE, &mut self.link).await {
                Ok(Ok(Err(error))) => {
                    tracing::warn!(%error, "the link failed while it stopped");
                }
                Ok(_ended) => {}
                Err(_elapsed) => {
                    tracing::warn!(
                        "the link's calls did not finish in time; its connections close"
                    );
                    self.link.abort();
                }
            }
        }
        self.runtime.shutdown().await;
        // The store finishes the queued writes before it closes.
        drop(self.store);
        tracing::info!("the worker stopped");
        drop(self.logging);
    }
}

/// The runtime of this host: the worker agent and its instructions, the
/// system tools under the ceiling with the worker's own folders protected,
/// and the lease vault for the provider's credential.
async fn start_runtime(
    options: &WorkerOptions,
    store: Store,
    leases: &Arc<LeaseVault>,
    system: SystemFacts,
    clock: Arc<dyn Clock>,
) -> Result<Runtime, StartError> {
    let config = &options.config;
    let client = HttpClient::new(Arc::clone(&clock)).map_err(StartError::HttpClient)?;
    let registry = ProviderRegistry::with_builtin(&client);
    let host = Arc::new(Host {
        name: config.name.clone(),
        purpose: config.purpose.clone(),
        facts: host::host_facts(&system),
    });
    let machine = MachineSettings {
        ceiling: Some(config.ceiling.clone()),
        name: Some(config.name.clone()),
        instructions_file: Some(config.host_instructions.clone()),
        task_slots: Some(config.max_parallel_tasks),
        assigned_agent: Some(WORKER_AGENT),
        accepts_assigner_full_access: config.accepts_master_full_access,
    };
    // Every access inside the worker's own folders asks the owner, so no
    // task changes the data, the configuration or the program that hold the
    // policy (T-18), unless the ceiling is in full access, where the owner
    // chose that nothing asks (ADR-0029).
    let protected = [
        &config.data_dir,
        &options.folders.config,
        &options.folders.program,
    ]
    .into_iter()
    .chain(
        config
            .file
            .as_ref()
            .and_then(|file| file.parent())
            .map(Path::to_path_buf)
            .as_ref(),
    )
    .map(|folder| protected_folder(folder))
    .collect();
    let vault: Arc<dyn CredentialVault> = Arc::<LeaseVault>::clone(leases);
    let deps = RuntimeDeps {
        store,
        client,
        registry,
        vault,
        clock,
        agents: agent::agents(),
        instructions: agent::instructions(&host),
        tools: ToolsDeps {
            registry: ToolRegistry::with_system_tools(),
            protected,
            mutations: MutationLock::new(),
            system: Arc::new(system),
        },
        machine,
        // A worker has no message box, so nothing is mentioned there.
        mentions: MentionRegistry::new(),
    };
    Runtime::start(deps, RuntimeConfig::default())
        .await
        .map_err(|error| {
            tracing::error!(error = %ErrorChain(&error), error.code = error.code(), "could not recover the tasks of the last run");
            StartError::Runtime(error)
        })
}

/// The data folder is readable only by the account that runs the worker
/// (security architecture, local data). On Windows, `service install` sets the
/// folder's access list; a worker started in a terminal keeps the folder's
/// existing permissions.
fn create_private_directory(directory: &Path) -> std::io::Result<()> {
    fs::create_dir_all(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
