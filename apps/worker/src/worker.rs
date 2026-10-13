//! Starting and stopping the worker: logging first, then the store.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mannie_logging::{ErrorChain, LogConfig, LoggingError, LoggingGuard, Redactor};
use mannie_store::{Store, StoreError, StoreOptions};
use mannie_types::clock::{Clock, SystemClock};

/// The file name of the worker's database in its data folder.
pub(crate) const DATABASE_FILE: &str = "mannie-worker.sqlite";

/// The start of the worker's log file names.
const LOG_FILE_PREFIX: &str = "mannie-worker";

/// What the worker needs to start.
#[derive(Debug, Clone)]
pub(crate) struct WorkerOptions {
    /// The folder for the database, its backups and the logs.
    pub(crate) data_directory: PathBuf,
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
}

/// A running worker. In M0 it holds its store and keeps logging.
#[derive(Debug)]
pub(crate) struct Worker {
    store: Store,
    logging: LoggingGuard,
}

impl Worker {
    /// Prepares the data folder, starts logging and opens the store.
    pub(crate) fn start(options: &WorkerOptions) -> Result<Self, StartError> {
        let data_directory = &options.data_directory;
        create_private_directory(data_directory).map_err(|source| StartError::DataDirectory {
            path: data_directory.clone(),
            source,
        })?;
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let config = LogConfig::new(
            data_directory.join("logs"),
            LOG_FILE_PREFIX,
            options.console_logging,
        );
        let logging = mannie_logging::init(&config, &Redactor::new(), Arc::clone(&clock))?;
        tracing::info!(
            version = env!("CARGO_PKG_VERSION"),
            data_directory = %data_directory.display(),
            "the worker is starting"
        );
        let database = data_directory.join(DATABASE_FILE);
        let store_options = StoreOptions::new(data_directory.join("backups"), clock);
        let store = Store::open(&database, &store_options).inspect_err(|error| {
            tracing::error!(error = %ErrorChain(error), error.code = error.code(), "could not open the database");
        })?;
        tracing::info!(database = %store.path().display(), schema_version = Store::schema_version(), "the worker started");
        Ok(Self { store, logging })
    }

    /// Closes the store and writes out the remaining log lines.
    pub(crate) fn stop(self) {
        tracing::info!("the worker is stopping");
        // The store finishes the queued writes before it closes.
        drop(self.store);
        tracing::info!("the worker stopped");
        drop(self.logging);
    }
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
