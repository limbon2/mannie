//! The core's state, which the commands share, and how it starts: logging
//! first, then the store.
//!
//! A store that does not open does not stop the app. The window opens, the
//! shell asks for the startup status, and it shows what failed and what the
//! owner can do, instead of the views.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use mannie_logging::{ErrorChain, LogConfig, LoggingGuard, Redactor};
use mannie_store::{Store, StoreError, StoreOptions};
use mannie_types::clock::{Clock, SystemClock};
use mannie_types::startup::StartupProblem;
use tauri::{App, Manager};

/// The file name of the master's database in its data folder.
const DATABASE_FILE: &str = "mannie.sqlite";

/// The core's state, shared by the commands.
#[derive(Debug)]
pub(crate) struct Core {
    store: Result<Store, StartupProblem>,
    /// Keeps the log files written until the app ends.
    _logging: Option<LoggingGuard>,
}

impl Core {
    /// The store, unless it did not open.
    pub(crate) fn store(&self) -> Option<&Store> {
        self.store.as_ref().ok()
    }

    /// Why the store did not open, if it did not.
    pub(crate) fn problem(&self) -> Option<&StartupProblem> {
        self.store.as_ref().err()
    }
}

/// Starts logging and opens the store. Failures are recorded for the UI
/// instead of stopping the app.
pub(crate) fn start(app: &mut App) {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let redactor = Redactor::new();
    let logging = start_logging(app, &redactor, &clock);
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "Mannie is starting");
    let store = open_store(app, clock);
    if let Err(problem) = &store {
        tracing::error!(?problem, "Mannie started without its database");
    }
    app.manage(Core {
        store,
        _logging: logging,
    });
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
