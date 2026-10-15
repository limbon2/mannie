//! The core's state, which the commands share, and how it starts and stops:
//! logging first, then the store, then the runtime, which marks the replies of
//! a previous process as interrupted before any command runs.
//!
//! A store that does not open does not stop the app. The window opens, the
//! shell asks for the startup status, and it shows what failed and what the
//! owner can do, instead of the views.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use mannie_logging::{ErrorChain, LogConfig, LoggingGuard, Redactor};
use mannie_provider::{HttpClient, ProviderRegistry};
use mannie_runtime::{
    AgentRegistry, InstructionRegistry, Runtime, RuntimeConfig, RuntimeDeps, ToolsDeps,
};
use mannie_store::{Store, StoreError, StoreOptions};
use mannie_tools::{MutationLock, SystemFacts, ToolRegistry};
use mannie_types::clock::{Clock, SystemClock};
use mannie_types::startup::StartupProblem;
use mannie_types::window::WindowSettings;
use tauri::{App, Manager};

use crate::credentials::KeyringVault;
use crate::ipc::subscriptions::Subscriptions;
use crate::window::requests::WindowRequests;

/// The file name of the master's database in its data folder.
const DATABASE_FILE: &str = "mannie.sqlite";

/// What runs once the store opened.
#[derive(Debug)]
struct Services {
    store: Store,
    runtime: Runtime,
}

#[derive(Debug)]
struct Inner {
    services: Result<Services, StartupProblem>,
    /// The window settings, kept in memory so that closing the window never
    /// waits for the database.
    window_settings: RwLock<WindowSettings>,
    window_requests: WindowRequests,
    subscriptions: Subscriptions,
    /// Keeps the log files written until the core shuts down.
    logging: Mutex<Option<LoggingGuard>>,
    /// The links that validation builds record instead of opening.
    #[cfg(feature = "validation")]
    opened_links: Mutex<Vec<String>>,
    /// The answer that the next folder dialog of a validation build gives
    /// instead of opening.
    #[cfg(feature = "validation")]
    folder_choice: Mutex<Option<FolderChoice>>,
}

/// What the next folder dialog of a validation build answers.
#[cfg(feature = "validation")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FolderChoice {
    /// The owner closed the dialog.
    Closed,
    /// The owner chose this folder.
    Folder(String),
}

/// The core's state, shared by the commands. Cloning is cheap; every clone
/// shares the same state.
#[derive(Debug, Clone)]
pub(crate) struct Core {
    inner: Arc<Inner>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Core {
    /// The store, unless it did not open.
    pub(crate) fn store(&self) -> Option<&Store> {
        self.inner
            .services
            .as_ref()
            .ok()
            .map(|services| &services.store)
    }

    /// The runtime, unless the core did not start.
    pub(crate) fn runtime(&self) -> Option<&Runtime> {
        self.inner
            .services
            .as_ref()
            .ok()
            .map(|services| &services.runtime)
    }

    /// Why the core did not start, if it did not.
    pub(crate) fn problem(&self) -> Option<&StartupProblem> {
        self.inner.services.as_ref().err()
    }

    /// Whether the store and the runtime run.
    pub(crate) fn is_started(&self) -> bool {
        self.inner.services.is_ok()
    }

    /// How many replies are active.
    pub(crate) fn active_run_count(&self) -> u32 {
        self.runtime().map_or(0, Runtime::active_run_count)
    }

    /// The window settings in memory.
    pub(crate) fn window_settings(&self) -> WindowSettings {
        self.inner
            .window_settings
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Replaces the window settings in memory, after they were saved.
    pub(crate) fn set_window_settings(&self, settings: WindowSettings) {
        *self
            .inner
            .window_settings
            .write()
            .unwrap_or_else(PoisonError::into_inner) = settings;
    }

    /// The questions to the window.
    pub(crate) fn window_requests(&self) -> &WindowRequests {
        &self.inner.window_requests
    }

    /// The UI's subscriptions.
    pub(crate) fn subscriptions(&self) -> &Subscriptions {
        &self.inner.subscriptions
    }

    /// The window's page loads, for the first time or again: the old page's
    /// subscriptions end, and the new page subscribes again.
    pub(crate) fn page_loaded(&self) {
        self.inner.subscriptions.cancel_all();
        self.inner.window_requests.unsubscribe();
    }

    /// Records a link that a validation build would have opened.
    #[cfg(feature = "validation")]
    pub(crate) fn record_opened_link(&self, url: String) {
        lock(&self.inner.opened_links).push(url);
    }

    /// The links that a validation build would have opened, in order.
    #[cfg(feature = "validation")]
    pub(crate) fn opened_links(&self) -> Vec<String> {
        lock(&self.inner.opened_links).clone()
    }

    /// Sets the answer of the next folder dialog of a validation build.
    #[cfg(feature = "validation")]
    pub(crate) fn set_folder_choice(&self, choice: FolderChoice) {
        *lock(&self.inner.folder_choice) = Some(choice);
    }

    /// Takes the answer of the next folder dialog, if a test set one.
    #[cfg(feature = "validation")]
    pub(crate) fn take_folder_choice(&self) -> Option<FolderChoice> {
        lock(&self.inner.folder_choice).take()
    }

    /// Stops the core: the runtime marks active replies as interrupted and
    /// waits for them, the store makes everything durable, and the logs are
    /// written out. Mannie quits afterwards.
    pub(crate) async fn shutdown(&self) {
        self.inner.subscriptions.cancel_all();
        if let Some(runtime) = self.runtime() {
            runtime.shutdown().await;
        }
        tracing::info!("Mannie stopped");
        drop(lock(&self.inner.logging).take());
    }
}

/// Starts logging, opens the store and starts the runtime. Failures are
/// recorded for the UI instead of stopping the app.
pub(crate) fn start(app: &App) -> Core {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let redactor = Redactor::new();
    let logging = start_logging(app, &redactor, &clock);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Mannie is starting");
    let services = open_store(app, Arc::clone(&clock))
        .and_then(|store| start_runtime(app, store, &redactor, clock));
    if let Err(problem) = &services {
        tracing::error!(?problem, "Mannie started without its database");
    }
    let window_settings = services
        .as_ref()
        .map(|services| load_window_settings(&services.store))
        .unwrap_or_default();
    Core {
        inner: Arc::new(Inner {
            services,
            window_settings: RwLock::new(window_settings),
            window_requests: WindowRequests::default(),
            subscriptions: Subscriptions::default(),
            logging: Mutex::new(logging),
            #[cfg(feature = "validation")]
            opened_links: Mutex::default(),
            #[cfg(feature = "validation")]
            folder_choice: Mutex::default(),
        }),
    }
}

/// Logging goes to the platform's log folder for Mannie. When it cannot
/// start, Mannie still runs; development builds report why on the console.
fn start_logging(app: &App, redactor: &Redactor, clock: &Arc<dyn Clock>) -> Option<LoggingGuard> {
    let started = app
        .path()
        .app_log_dir()
        .map_err(|error| error.to_string())
        .and_then(|directory| {
            let config = LogConfig::new(directory, "mannie", cfg!(debug_assertions));
            mannie_logging::init(&config, redactor, Arc::clone(clock))
                .map_err(|error| ErrorChain(&error).to_string())
        });
    match started {
        Ok(guard) => Some(guard),
        Err(error) => {
            #[cfg(debug_assertions)]
            #[expect(
                clippy::print_stderr,
                reason = "without a logger, the console is the only place to report this"
            )]
            {
                eprintln!("logging could not start: {error}");
            }
            drop(error);
            None
        }
    }
}

/// Opens the master's database in the platform's local data folder.
fn open_store(app: &App, clock: Arc<dyn Clock>) -> Result<Store, StartupProblem> {
    let data_directory = app.path().app_local_data_dir().map_err(|error| {
        tracing::error!(%error, "could not determine the data folder");
        StartupProblem::DataDirectoryUnavailable
    })?;
    let database = data_directory.join(DATABASE_FILE);
    let backups = data_directory.join("backups");
    create_private_directory(&data_directory).map_err(|error| {
        tracing::error!(error = %ErrorChain(&error), directory = %data_directory.display(), "could not create the data folder");
        StartupProblem::DatabaseUnavailable { path: database.display().to_string() }
    })?;
    Store::open(&database, &StoreOptions::new(backups.clone(), clock)).map_err(|error| {
        tracing::error!(error = %ErrorChain(&error), error.code = error.code(), "could not open the database");
        startup_problem(&error, &database, &backups)
    })
}

/// Starts the runtime, which first marks the replies of a previous process as
/// interrupted. The credential store opens on first use.
fn start_runtime(
    app: &App,
    store: Store,
    redactor: &Redactor,
    clock: Arc<dyn Clock>,
) -> Result<Services, StartupProblem> {
    let client = HttpClient::new(Arc::clone(&clock)).map_err(|error| {
        tracing::error!(error = %ErrorChain(&error), "could not set up the HTTP client for model providers");
        StartupProblem::HttpClientUnavailable
    })?;
    let registry = ProviderRegistry::with_builtin(&client);
    let vault = Arc::new(KeyringVault::new(
        &app.config().identifier,
        redactor.clone(),
    ));
    let deps = RuntimeDeps {
        store: store.clone(),
        client,
        registry,
        vault,
        clock,
        agents: AgentRegistry::with_builtin(),
        instructions: InstructionRegistry::with_builtin(),
        tools: tools(app),
    };
    let runtime = tauri::async_runtime::block_on(Runtime::start(deps, RuntimeConfig::default()))
        .map_err(|error| {
            tracing::error!(error = %ErrorChain(&error), error.code = error.code(), "could not recover the replies of the last run");
            StartupProblem::DatabaseUnavailable {
                path: store.path().display().to_string(),
            }
        })?;
    Ok(Services { store, runtime })
}

/// The system tools of the master. Mannie's data and log folders are
/// protected: every access inside them asks the owner, so no grant or setting
/// lets the model change the data that holds the policy (T-18).
fn tools(app: &App) -> ToolsDeps {
    let folders = [app.path().app_local_data_dir(), app.path().app_log_dir()];
    let protected = folders
        .into_iter()
        .filter_map(|folder| {
            folder
                .inspect_err(|error| tracing::warn!(%error, "a folder of Mannie's own is unknown"))
                .ok()
        })
        .map(|folder| protected_folder(&folder))
        .collect();
    ToolsDeps {
        registry: ToolRegistry::with_system_tools(),
        protected,
        mutations: MutationLock::new(),
        system: Arc::new(SystemFacts::detect()),
    }
}

/// A folder of Mannie's own, in the canonical form that the policy compares.
/// It is created first, so that it has one.
fn protected_folder(folder: &Path) -> PathBuf {
    if let Err(error) = create_private_directory(folder) {
        tracing::warn!(error = %ErrorChain(&error), "could not create a folder of Mannie's own");
    }
    match mannie_tools::canonical_folder(folder) {
        Ok(canonical) => PathBuf::from(canonical),
        Err(error) => {
            tracing::warn!(
                error.code = error.code(),
                "could not resolve a folder of Mannie's own; it is protected as it is named"
            );
            folder.to_path_buf()
        }
    }
}

/// The stored window settings, or the defaults when they cannot be read.
fn load_window_settings(store: &Store) -> WindowSettings {
    tauri::async_runtime::block_on(store.load_settings::<WindowSettings>()).unwrap_or_else(|error| {
        tracing::warn!(error = %ErrorChain(&error), error.code = error.code(), "could not read the window settings; closing asks");
        WindowSettings::default()
    })
}

/// The data folder is readable only by the account that runs Mannie
/// (security architecture, local data).
fn create_private_directory(directory: &Path) -> std::io::Result<()> {
    fs::create_dir_all(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn startup_problem(error: &StoreError, database: &Path, backups: &Path) -> StartupProblem {
    let path = database.display().to_string();
    match error {
        StoreError::SchemaTooNew {
            database: database_version,
            supported,
            ..
        } => StartupProblem::DatabaseTooNew {
            path,
            database_version: *database_version,
            supported_version: *supported,
        },
        StoreError::Migration { version, .. } => StartupProblem::MigrationFailed {
            path,
            version: *version,
            backup_directory: backups.display().to_string(),
        },
        _ => StartupProblem::DatabaseUnavailable { path },
    }
}
