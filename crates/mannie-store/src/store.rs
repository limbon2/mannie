//! Opening the store: the schema check, the copy before migrating, the
//! migrations, and the writer and readers that serve the open database.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use mannie_types::clock::Clock;
use rusqlite::{Connection, OpenFlags, Transaction};

use crate::Durability;
use crate::error::{DatabaseError, StoreError};
use crate::readers::Readers;
use crate::schema::{self, Migration};
use crate::writer::Writer;

/// How long a connection waits for a lock before it reports the database as
/// busy. Only one process opens the database, so a lock is never held long.
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// The default number of read-only connections.
const DEFAULT_READERS: usize = 4;

/// How to open the store.
#[derive(Debug, Clone)]
pub struct StoreOptions {
    /// Where the store copies the database before it migrates it.
    pub backup_directory: PathBuf,
    /// How many read-only connections serve reads.
    pub readers: usize,
    /// The clock for timestamps and backup names.
    pub clock: Arc<dyn Clock>,
}

impl StoreOptions {
    /// Options with the default number of readers.
    pub fn new(backup_directory: PathBuf, clock: Arc<dyn Clock>) -> Self {
        Self {
            backup_directory,
            readers: DEFAULT_READERS,
            clock,
        }
    }
}

/// The open database of one machine.
///
/// Cloning is cheap: every clone shares the writer thread and the readers.
#[derive(Debug, Clone)]
pub struct Store {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    path: PathBuf,
    writer: Writer,
    readers: Arc<Readers>,
    clock: Arc<dyn Clock>,
}

impl Store {
    /// Opens the database at `path`, creating it when it does not exist, and
    /// brings its schema up to date.
    ///
    /// A database with a newer schema than this binary knows is refused and
    /// left untouched. Before migrating an existing database, the store copies
    /// it into the backup directory.
    ///
    /// This blocks on file I/O: call it at startup or from a blocking task.
    pub fn open(path: &Path, options: &StoreOptions) -> Result<Self, StoreError> {
        Self::open_with_migrations(path, options, schema::MIGRATIONS)
    }

    /// Opens the store with a given list of migrations. Tests use it to stand
    /// in for older and newer binaries.
    pub(crate) fn open_with_migrations(
        path: &Path,
        options: &StoreOptions,
        migrations: &[Migration],
    ) -> Result<Self, StoreError> {
        let _span = tracing::info_span!("open_store", path = %path.display()).entered();
        let supported = schema::supported_version(migrations);
        let exists = fs::exists(path).map_err(|source| StoreError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if exists {
            refuse_newer_schema(path, supported)?;
        } else if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| StoreError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }

        let open_failed = |source: rusqlite::Error| StoreError::Open {
            path: path.to_path_buf(),
            source: DatabaseError(source),
        };
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut connection = Connection::open_with_flags(path, flags).map_err(open_failed)?;
        restrict_permissions(path)?;
        configure_connection(&connection).map_err(open_failed)?;
        let journal_mode = enable_wal(&connection).map_err(open_failed)?;
        if !journal_mode.eq_ignore_ascii_case("wal") {
            return Err(StoreError::WalUnavailable {
                path: path.to_path_buf(),
                journal_mode,
            });
        }

        let version = schema::schema_version(&connection).map_err(open_failed)?;
        if version > supported {
            return Err(StoreError::SchemaTooNew {
                path: path.to_path_buf(),
                database: version,
                supported,
            });
        }
        if version < supported {
            if version > 0 {
                let backup = schema::back_up(
                    &connection,
                    path,
                    &options.backup_directory,
                    version,
                    options.clock.now(),
                )?;
                tracing::info!(backup = %backup.display(), from = version, "copied the database before migrating it");
            }
            schema::migrate(&mut connection, migrations, version)?;
            tracing::info!(from = version, to = supported, "migrated the database");
        }

        let readers = Arc::new(Readers::open(path, options.readers.max(1))?);
        let writer = Writer::start(connection).map_err(|source| StoreError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Self {
            inner: Arc::new(Inner {
                path: path.to_path_buf(),
                writer,
                readers,
                clock: Arc::clone(&options.clock),
            }),
        })
    }

    /// The database file.
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// The schema version this binary migrates databases to.
    pub fn schema_version() -> u32 {
        schema::supported_version(schema::MIGRATIONS)
    }

    pub(crate) fn clock(&self) -> &dyn Clock {
        self.inner.clock.as_ref()
    }

    /// Runs `work` in a transaction on the writer thread. The future resolves
    /// once the transaction has committed with the given durability.
    pub(crate) async fn write<T, F>(&self, durability: Durability, work: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> rusqlite::Result<T> + Send + 'static,
    {
        self.inner.writer.write(durability, work).await
    }

    /// Runs `work` on a read-only connection.
    pub(crate) async fn read<T, F>(&self, work: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        self.inner.readers.read(work).await
    }
}

/// Reads the schema version through a read-only connection and refuses a
/// database newer than this binary, before anything could change the file.
fn refuse_newer_schema(path: &Path, supported: u32) -> Result<(), StoreError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let version = Connection::open_with_flags(path, flags)
        .and_then(|connection| schema::schema_version(&connection))
        .map_err(|source| StoreError::Open {
            path: path.to_path_buf(),
            source: DatabaseError(source),
        })?;
    if version > supported {
        tracing::error!(
            database = version,
            supported,
            "refused a database written by a newer version of Mannie"
        );
        return Err(StoreError::SchemaTooNew {
            path: path.to_path_buf(),
            database: version,
            supported,
        });
    }
    Ok(())
}

/// Settings that every connection needs.
pub(crate) fn configure_connection(connection: &Connection) -> rusqlite::Result<()> {
    connection.busy_timeout(BUSY_TIMEOUT)?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

/// Switches the database to write-ahead logging and returns the journal mode
/// SQLite reports afterwards. The mode is stored in the file, so it only
/// changes on the first open.
fn enable_wal(connection: &Connection) -> rusqlite::Result<String> {
    connection.pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))
}

/// Makes the database readable only by its owner (security architecture,
/// local data). SQLite gives the WAL and shared-memory files the same mode.
#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|source| StoreError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// On Windows the folder's access control list keeps other users out: the
/// user's profile for the desktop app, and the folder that the worker's
/// installer restricts to the service account.
#[cfg(not(unix))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "the Unix version of this function can fail"
)]
fn restrict_permissions(_path: &Path) -> Result<(), StoreError> {
    Ok(())
}

#[cfg(test)]
mod tests;
