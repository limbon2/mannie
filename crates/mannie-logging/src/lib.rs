//! Logging for the desktop app and the worker (engineering standards,
//! observability).
//!
//! - `tracing` writes JSON lines to a daily rotating file in the machine's log
//!   folder. Files are kept for 14 days, and each day's file stops growing at a
//!   size limit.
//! - Every line passes through a [`Redactor`] before it is written, and so does
//!   the crash file that the panic hook writes.
//! - Development builds, and a worker running in a terminal, also log to the
//!   console.
//! - Nothing leaves the machine: there is no telemetry and no crash reporting
//!   service.
//!
//! Mannie's own crates log at `info` and above, and other crates at `warn` and
//! above. The `MANNIE_LOG` environment variable replaces this with a filter in
//! `tracing-subscriber`'s `EnvFilter` syntax, for example `MANNIE_LOG=debug`.

mod cap;
mod chain;
mod panic;
mod redact;

use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mannie_types::clock::Clock;
use tracing::Level;
use tracing_appender::non_blocking::{NonBlockingBuilder, WorkerGuard};
use tracing_appender::rolling::{InitError, RollingFileAppender, Rotation};
use tracing_subscriber::filter::{EnvFilter, filter_fn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::{SubscriberInitExt, TryInitError};
use tracing_subscriber::{Layer, Registry, fmt};

pub use chain::ErrorChain;
pub use redact::{REDACTED, Redactor};

/// The environment variable that overrides the default filter.
pub const FILTER_VARIABLE: &str = "MANNIE_LOG";

/// How many daily files are kept.
const RETAINED_DAYS: usize = 14;

/// The default size limit of one day's file: 32 MiB.
const DEFAULT_MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;

/// Where and how to log.
#[derive(Debug, Clone)]
pub struct LogConfig {
    /// The machine's log folder.
    pub directory: PathBuf,
    /// The start of every log file name, such as `mannie` or `mannie-worker`.
    pub file_prefix: String,
    /// Whether lines also go to standard error.
    pub console: bool,
    /// The size at which a day's file stops growing.
    pub max_file_bytes: u64,
}

impl LogConfig {
    /// A configuration with the default size limit.
    pub fn new(directory: PathBuf, file_prefix: impl Into<String>, console: bool) -> Self {
        Self {
            directory,
            file_prefix: file_prefix.into(),
            console,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
        }
    }
}

/// Keeps logging running. Dropping it writes out the lines still queued, so
/// the composition root keeps it until the process ends.
#[must_use = "logging stops writing files when the guard is dropped"]
#[derive(Debug)]
pub struct LoggingGuard {
    _flush_on_drop: WorkerGuard,
}

/// Why logging could not start.
#[derive(Debug, thiserror::Error)]
pub enum LoggingError {
    /// The log folder could not be created.
    #[error("could not create the log folder {path}")]
    Directory {
        /// The folder.
        path: PathBuf,
        /// The cause.
        #[source]
        source: std::io::Error,
    },
    /// The daily log file could not be opened.
    #[error("could not open the daily log file")]
    File(#[from] InitError),
    /// The `MANNIE_LOG` filter is not valid.
    #[error("the {FILTER_VARIABLE} filter is not valid")]
    Filter(#[from] tracing_subscriber::filter::ParseError),
    /// Another logger is already installed in this process.
    #[error("a logger is already installed")]
    AlreadyInstalled(#[from] TryInitError),
}

impl LoggingError {
    /// The stable code of the error.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Directory { .. } => "logging.directory",
            Self::File(_) => "logging.file",
            Self::Filter(_) => "logging.filter",
            Self::AlreadyInstalled(_) => "logging.already_installed",
        }
    }
}

/// Starts logging for the process and installs the panic hook.
///
/// Call it once, first thing in `main`. Secrets that the process reads later
/// are registered with the same `redactor`.
pub fn init(
    config: &LogConfig,
    redactor: &Redactor,
    clock: Arc<dyn Clock>,
) -> Result<LoggingGuard, LoggingError> {
    create_private_directory(&config.directory)?;

    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(&config.file_prefix)
        .filename_suffix("log")
        .max_log_files(RETAINED_DAYS)
        .build(&config.directory)?;
    let capped = cap::CappedWriter::new(
        appender,
        config.max_file_bytes,
        Arc::clone(&clock),
        config.directory.clone(),
        config.file_prefix.clone(),
    );
    // A full queue drops lines instead of blocking the thread that logs.
    let (file_writer, flush_on_drop) = NonBlockingBuilder::default()
        .lossy(true)
        .thread_name("mannie-log-writer")
        .finish(capped);

    let file_layer = fmt::layer()
        .json()
        .with_current_span(true)
        .with_span_list(true)
        .with_writer(redact::RedactingMakeWriter::new(
            file_writer,
            redactor.clone(),
        ));
    let console_layer = config.console.then(|| {
        fmt::layer()
            .with_ansi(std::io::stderr().is_terminal())
            .with_writer(redact::RedactingMakeWriter::new(
                std::io::stderr,
                redactor.clone(),
            ))
    });

    tracing_subscriber::registry()
        .with(filter()?)
        .with(file_layer)
        .with(console_layer)
        .try_init()?;
    panic::install_panic_hook(
        config.directory.clone(),
        redactor.clone(),
        clock,
        config.console,
    );
    Ok(LoggingGuard {
        _flush_on_drop: flush_on_drop,
    })
}

/// The filter from `MANNIE_LOG`, or the default one.
fn filter() -> Result<Box<dyn Layer<Registry> + Send + Sync>, LoggingError> {
    if let Ok(directives) = std::env::var(FILTER_VARIABLE) {
        return Ok(Box::new(EnvFilter::try_new(directives)?));
    }
    Ok(Box::new(filter_fn(|metadata| {
        let is_mannie = metadata.target().starts_with("mannie") || metadata.target() == "panic";
        let most_verbose = if is_mannie { Level::INFO } else { Level::WARN };
        *metadata.level() <= most_verbose
    })))
}

/// Creates the log folder, readable only by the account that runs Mannie
/// (security architecture, local data).
fn create_private_directory(directory: &Path) -> Result<(), LoggingError> {
    let failed = |source| LoggingError::Directory {
        path: directory.to_path_buf(),
        source,
    };
    fs::create_dir_all(directory).map_err(failed)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).map_err(failed)?;
    }
    Ok(())
}
