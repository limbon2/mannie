//! The core's state, which the commands share, and how it starts and stops:
//! logging first, then the list of plugins with their manifests and the
//! store with the plugins' modules, then the runtime, which marks the
//! replies of a previous process as interrupted before any command runs,
//! then the plugin host, which starts the plugins that the owner turned on
//! with the services that their manifests declare (ADR-0030, ADR-0034).
//!
//! A store that does not open does not stop the app. The window opens, the
//! shell asks for the startup status, and it shows what failed and what the
//! owner can do, instead of the views.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use mannie_logging::{ErrorChain, LogConfig, LoggingGuard, Redactor};
use mannie_plugin_sdk::context::{HostInfo, HostProfile};
use mannie_plugin_sdk::host::ServiceProvider;
use mannie_plugin_sdk::plugin::PluginEntry;
use mannie_plugins::{PluginHost, PluginHostDeps, PluginList};
use mannie_provider::{HttpClient, ProviderRegistry};
use mannie_runtime::{
    AgentRegistry, FileMentions, InstructionRegistry, MachineSettings, MentionRegistry, Runtime,
    RuntimeConfig, RuntimeDeps, ToolsDeps,
};
use mannie_store::{Store, StoreError, StoreOptions};
use mannie_tools::{MutationLock, SystemFacts, ToolRegistry};
use mannie_types::clock::{Clock, SystemClock};
use mannie_types::startup::StartupProblem;
use mannie_types::window::WindowSettings;
use tauri::{App, Manager};

use crate::credentials::{KeyringVault, SecretsProvider};
use crate::file_dialog::FileDialogProvider;
use crate::ipc::subscriptions::Subscriptions;
use crate::window::requests::WindowRequests;

/// The file name of the master's database in its data folder.
const DATABASE_FILE: &str = "mannie.sqlite";

/// What runs once the store opened.
#[derive(Debug)]
struct Services {
    store: Store,
    runtime: Runtime,
    plugins: PluginHost,
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
    /// The system's file dialog, whose next answer a validation build sets.
    /// It exists before the store opens, so a test can set the answer even
    /// when the store did not open.
    #[cfg(feature = "validation")]
    file_dialog: Arc<FileDialogProvider>,
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

    /// The plugin host, unless the core did not start.
    pub(crate) fn plugins(&self) -> Option<&PluginHost> {
        self.inner
            .services
            .as_ref()
            .ok()
            .map(|services| &services.plugins)
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

    /// Sets the files that the next file dialog of a validation build gives.
    #[cfg(feature = "validation")]
    pub(crate) fn set_file_choice(&self, files: Vec<String>) {
        self.inner.file_dialog.set_choice(files);
    }

    /// Stops the core: the runtime marks active replies as interrupted and
    /// waits for them, and lets the tasks of plugins' executors run on; the
    /// plugins that are on stop and keep the owner's choices; the store makes
    /// everything durable, and the logs are written out. Mannie quits
    /// afterwards.
    pub(crate) async fn shutdown(&self) {
        self.inner.subscriptions.cancel_all();
        if let Some(runtime) = self.runtime() {
            runtime.shutdown().await;
        }
        if let Some(plugins) = self.plugins() {
            plugins.shutdown().await;
        }
        tracing::info!("Mannie stopped");
        drop(lock(&self.inner.logging).take());
    }
}

/// Starts logging, opens the store with the modules of `plugins` and starts
/// the runtime and the plugin host. Failures are recorded for the UI instead
/// of stopping the app.
pub(crate) fn start(app: &App, plugins: Vec<PluginEntry>) -> Core {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let redactor = Redactor::new();
    let logging = start_logging(app, &redactor, &clock);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Mannie is starting");
    let file_dialog = Arc::new(FileDialogProvider::new(app.handle().clone()));
    let services = open_store(app, Arc::clone(&clock), plugins).and_then(|(store, plugins)| {
        let dialog = Arc::clone(&file_dialog);
        start_runtime(app, store, plugins, &redactor, dialog, clock)
    });
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
            file_dialog,
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

/// How this build was made, as the plugins learn it (ADR-0034, decision 5):
/// validation builds drive the end-to-end tests, which see a stopped worker
/// come back sooner, debug builds are development builds, and every other
/// build is a release.
fn host_profile() -> HostProfile {
    if cfg!(feature = "validation") {
        HostProfile::Validation
    } else if cfg!(debug_assertions) {
        HostProfile::Development
    } else {
        HostProfile::Release
    }
}

/// Opens the master's database in the platform's local data folder, with
/// the modules of the plugins, whose migrations run whether or not the
/// plugins are on (ADR-0030, decision 5).
fn open_store(
    app: &App,
    clock: Arc<dyn Clock>,
    plugins: Vec<PluginEntry>,
) -> Result<(Store, PluginList), StartupProblem> {
    let data_directory = app.path().app_local_data_dir().map_err(|error| {
        tracing::error!(%error, "could not determine the data folder");
        StartupProblem::DataDirectoryUnavailable
    })?;
    let database = data_directory.join(DATABASE_FILE);
    let backups = data_directory.join("backups");
    // A list that the host refuses is a fault of the build, which the test
    // of the bundled plugins catches, so it stops the store as a host error
    // does (the plugin SDK's step 1, D20).
    let plugins = PluginList::new(plugins).map_err(|error| {
        tracing::error!(error = %ErrorChain(&error), "could not take the list of plugins");
        StartupProblem::DatabaseUnavailable {
            path: database.display().to_string(),
        }
    })?;
    create_private_directory(&data_directory).map_err(|error| {
        tracing::error!(error = %ErrorChain(&error), directory = %data_directory.display(), "could not create the data folder");
        StartupProblem::DatabaseUnavailable { path: database.display().to_string() }
    })?;
    let options =
        StoreOptions::new(backups.clone(), clock).with_plugin_modules(plugins.store_modules());
    let store = Store::open(&database, &options).map_err(|error| {
        tracing::error!(error = %ErrorChain(&error), error.code = error.code(), "could not open the database");
        startup_problem(&error, &database, &backups)
    })?;
    Ok((store, plugins))
}

/// Starts the runtime, which first marks the replies of a previous process
/// as interrupted, and the plugin host. The credential store opens on first
/// use, for a provider's credential or a plugin's entry.
fn start_runtime(
    app: &App,
    store: Store,
    plugins: PluginList,
    redactor: &Redactor,
    file_dialog: Arc<FileDialogProvider>,
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
        vault: Arc::<KeyringVault>::clone(&vault),
        clock: Arc::clone(&clock),
        agents: AgentRegistry::with_builtin(),
        instructions: InstructionRegistry::with_builtin(),
        tools: tools(app),
        machine: MachineSettings::default(),
        mentions: mentions(),
    };
    let runtime = tauri::async_runtime::block_on(Runtime::start(deps, RuntimeConfig::default()))
        .map_err(|error| {
            tracing::error!(error = %ErrorChain(&error), error.code = error.code(), "could not recover the replies of the last run");
            StartupProblem::DatabaseUnavailable {
                path: store.path().display().to_string(),
            }
        })?;
    // Each plugin gets the services that its manifest declares and no other,
    // which these providers bind to it (ADR-0034, decision 5): the
    // runtime's executor host and credentials, the store's modules, the
    // plugin's own entries of the credential store, and the file dialog.
    let file_dialog: Arc<dyn ServiceProvider> = file_dialog;
    let mut services: Vec<Arc<dyn ServiceProvider>> = runtime.service_providers();
    services.extend(mannie_store::service_providers(&store));
    services.push(Arc::new(SecretsProvider::new(vault)));
    services.push(file_dialog);
    let plugins = PluginHost::new(PluginHostDeps {
        store: store.clone(),
        plugins,
        providers: services,
        consumers: runtime.contribution_consumers(),
        clock,
        host: HostInfo::new(env!("CARGO_PKG_VERSION"), host_profile()),
    })
    .map_err(|error| {
        tracing::error!(error = %ErrorChain(&error), "could not take the list of plugins");
        StartupProblem::DatabaseUnavailable {
            path: store.path().display().to_string(),
        }
    })?;
    // The plugins start after the core's recovery, in the background, so the
    // window never waits for them; the Plugins page shows each one starting.
    let starting = plugins.clone();
    tauri::async_runtime::spawn(async move { starting.start_chosen().await });
    Ok(Services {
        store,
        runtime,
        plugins,
    })
}

/// The core's mention providers (ADR-0032, decision 9): the files and folders
/// of a conversation's project.
fn mentions() -> MentionRegistry {
    let mut mentions = MentionRegistry::new();
    // The registry starts empty, so the ID is free.
    if let Err(error) = mentions.register(Arc::new(FileMentions::new())) {
        tracing::error!(%error, "could not register the mentions of files");
    }
    mentions
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
        }
        | StoreError::ModuleTooNew {
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
