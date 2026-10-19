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
use crate::plugin_store::{self, PluginModule, RegisteredModule};
use crate::readers::Readers;
use crate::schema::{self, CORE_MODULES, Schema, StoreModule};
use crate::writer::{TransactError, Writer};

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
    /// The modules of the composition root's plugins, which follow the core's
    /// modules (ADR-0030, decision 5; ADR-0034, decision 7).
    pub plugin_modules: Vec<PluginModule>,
}

impl StoreOptions {
    /// Options with the default number of readers and only the core's
    /// modules.
    pub fn new(backup_directory: PathBuf, clock: Arc<dyn Clock>) -> Self {
        Self {
            backup_directory,
            readers: DEFAULT_READERS,
            clock,
            plugin_modules: Vec::new(),
        }
    }

    /// Adds the modules of plugins, in the order that a new database applies
    /// them.
    #[must_use]
    pub fn with_plugin_modules(mut self, modules: impl IntoIterator<Item = PluginModule>) -> Self {
        self.plugin_modules.extend(modules);
        self
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
    /// The names of the modules that the store opened with.
    modules: Vec<&'static str>,
    /// The plugins' modules with their listed statements.
    plugin_modules: Vec<RegisteredModule>,
}

impl Store {
    /// Opens the database at `path`, creating it when it does not exist, and
    /// brings the schema of the core's modules and of the options' modules up
    /// to date.
    ///
    /// A database in which a module has a newer schema than this binary knows
    /// is refused and left untouched. Before migrating an existing database,
    /// the store copies it into the backup directory.
    ///
    /// This blocks on file I/O: call it at startup or from a blocking task.
    pub fn open(path: &Path, options: &StoreOptions) -> Result<Self, StoreError> {
        let mut modules: Vec<StoreModule> = CORE_MODULES.to_vec();
        for plugin in &options.plugin_modules {
            modules.push(plugin_store::schema_module(plugin)?);
        }
        Self::open_registering(path, options, &modules, &options.plugin_modules)
    }

    /// Opens the store with the given modules, in the order that a new
    /// database applies them. Tests use it to stand in for older and newer
    /// binaries.
    #[cfg(test)]
    pub(crate) fn open_with_modules(
        path: &Path,
        options: &StoreOptions,
        modules: &[StoreModule],
    ) -> Result<Self, StoreError> {
        Self::open_registering(path, options, modules, &[])
    }

    /// Opens the store with `modules`, then prepares the listed statements of
    /// each of `plugins`, whose modules are among `modules`.
    fn open_registering(
        path: &Path,
        options: &StoreOptions,
        modules: &[StoreModule],
        plugins: &[PluginModule],
    ) -> Result<Self, StoreError> {
        let _span = tracing::info_span!("open_store", path = %path.display()).entered();
        let mut names: Vec<&'static str> = Vec::with_capacity(modules.len());
        for module in modules {
            if names.contains(&module.name) {
                return Err(StoreError::DuplicateModule {
                    module: module.name,
                });
            }
            names.push(module.name);
        }
        let exists = fs::exists(path).map_err(|source| StoreError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if exists {
            refuse_newer_schema(path, modules)?;
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

        let schema = Schema::read(&connection).map_err(open_failed)?;
        schema.refuse_newer(path, modules)?;
        if schema.needs_changes(modules) {
            if schema.user_version > 0 {
                let backup = schema::back_up(
                    &connection,
                    path,
                    &options.backup_directory,
                    schema.user_version,
                    options.clock.now(),
                )?;
                tracing::info!(backup = %backup.display(), from = schema.user_version, "copied the database before migrating it");
            }
            schema::migrate(&mut connection, &schema, modules, options.clock.now())?;
            tracing::info!(from = schema.user_version, "migrated the database");
        }

        // The listed statements prepare against the migrated schema, on the
        // read-write connection, before the store serves anything.
        let plugin_modules = plugins
            .iter()
            .map(|plugin| plugin_store::register(&connection, plugin))
            .collect();
        let readers = Arc::new(Readers::open(path, options.readers.max(1))?);
        let writer = Writer::start(connection, path)?;
        Ok(Self {
            inner: Arc::new(Inner {
                path: path.to_path_buf(),
                writer,
                readers,
                clock: Arc::clone(&options.clock),
                modules: names,
                plugin_modules,
            }),
        })
    }

    /// Creates the database as a binary up to 0.2.0 left it at schema
    /// `version` of the single list of migrations, with the core's modules
    /// and those in `options`, and serves it without migrating it further.
    /// Tests use it to seed an older database through the repositories
    /// before they open it again.
    ///
    /// # Errors
    ///
    /// When the database cannot be created.
    #[cfg(any(test, feature = "testing"))]
    pub fn open_as_before_m5(
        path: &Path,
        options: &StoreOptions,
        version: u32,
    ) -> Result<Self, StoreError> {
        let open_failed = |source: rusqlite::Error| StoreError::Open {
            path: path.to_path_buf(),
            source: DatabaseError(source),
        };
        let mut modules: Vec<StoreModule> = CORE_MODULES.to_vec();
        for plugin in &options.plugin_modules {
            modules.push(plugin_store::schema_module(plugin)?);
        }
        let mut connection = Connection::open(path).map_err(open_failed)?;
        configure_connection(&connection).map_err(open_failed)?;
        enable_wal(&connection).map_err(open_failed)?;
        schema::migrate_as_before_m5(&mut connection, &modules, version).map_err(open_failed)?;
        let readers = Arc::new(Readers::open(path, options.readers.max(1))?);
        let writer = Writer::start(connection, path)?;
        Ok(Self {
            inner: Arc::new(Inner {
                path: path.to_path_buf(),
                writer,
                readers,
                clock: Arc::clone(&options.clock),
                modules: modules.iter().map(|module| module.name).collect(),
                // A database as 0.2.0 left it serves no plugin: tests seed it
                // with SQL that no module lists.
                plugin_modules: Vec::new(),
            }),
        })
    }

    /// The database file.
    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// The names of the modules that the store opened with.
    pub fn modules(&self) -> &[&'static str] {
        &self.inner.modules
    }

    /// The plugins' modules with their listed statements.
    pub(crate) fn plugin_modules(&self) -> &[RegisteredModule] {
        &self.inner.plugin_modules
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

    /// Runs `work` in a transaction that it may refuse: when `work` returns an
    /// error, nothing is written and the error is returned.
    pub(crate) async fn transact<T, E, F>(
        &self,
        durability: Durability,
        work: F,
    ) -> Result<T, TransactError<E>>
    where
        T: Send + 'static,
        E: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> Result<T, TransactError<E>> + Send + 'static,
    {
        self.inner.writer.transact(durability, work).await
    }

    /// Runs `work` on the writer's connection outside of a transaction.
    pub(crate) async fn maintain<T, F>(&self, work: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        self.inner.writer.maintain(work).await
    }

    /// Waits until every write queued before this call is committed, then
    /// copies the write-ahead log into the database file and empties the log.
    ///
    /// SQLite syncs the log before it copies it, so afterwards the batches of
    /// streamed text that were committed with normal durability survive a power
    /// loss too. Shutting down calls it last. Deleting a conversation calls it
    /// so that the deleted text does not stay in the log.
    pub async fn flush(&self) -> Result<(), StoreError> {
        let busy = self
            .maintain(|connection| {
                connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                    row.get::<_, i64>(0)
                })
            })
            .await?;
        if busy != 0 {
            // A read still used an older snapshot after the busy timeout, so
            // part of the log stays until the next checkpoint copies it.
            tracing::warn!("the write-ahead log could not be emptied while a read was running");
        }
        Ok(())
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

/// Reads the schema through a read-only connection and refuses a database
/// that a newer binary migrated further, before anything could change the
/// file.
fn refuse_newer_schema(path: &Path, modules: &[StoreModule]) -> Result<(), StoreError> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let schema = Connection::open_with_flags(path, flags)
        .and_then(|connection| Schema::read(&connection))
        .map_err(|source| StoreError::Open {
            path: path.to_path_buf(),
            source: DatabaseError(source),
        })?;
    schema.refuse_newer(path, modules)
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
