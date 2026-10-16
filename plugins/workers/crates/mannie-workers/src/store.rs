//! The Workers plugin's tables (docs/features/workers/pairing-and-link.md,
//! data; ADR-0030, decision 9): the workers that the master paired with.
//!
//! The module's migrations run at startup with the core's, whether or not
//! Workers is on, so the schema never depends on the plugin's state. Before
//! M5 they were migrations 19, 20 and 22 of the store's single list, whose
//! numbers carry over the version of an older database (ADR-0030,
//! decision 5). The plugin's code reaches its tables through the store's
//! writer and readers, opened to this module.

use mannie_link::host::WorkerDetails;
use mannie_store::sql::{ErrorCode, params};
use mannie_store::{Durability, Migration, ModuleStore, StoreError, StoreModule, TransactError};
use mannie_types::clock::UtcMillis;
use mannie_types::ids::WorkerId;

/// The plugin's module of the store: the tables that `workers_` prefixes.
pub const WORKERS_MODULE: StoreModule = StoreModule {
    name: "workers",
    migrations: &[
        Migration {
            version: 1,
            legacy_version: Some(19),
            description: "create the paired workers table",
            sql: include_str!("store/migrations/0019_create_paired_workers.sql"),
        },
        Migration {
            version: 2,
            legacy_version: Some(20),
            description: "create the table of deletions that wait for a worker",
            sql: include_str!("store/migrations/0020_create_worker_deletions.sql"),
        },
        Migration {
            version: 3,
            legacy_version: Some(22),
            description: "add the name that the owner gives a worker",
            sql: include_str!("store/migrations/0022_add_worker_chosen_names.sql"),
        },
        Migration {
            version: 4,
            legacy_version: None,
            description: "move the deletions that wait for a worker into the core's outbox",
            sql: include_str!("store/migrations/0028_move_worker_deletions_to_the_outbox.sql"),
        },
    ],
};

/// The version of the JSON documents this module writes.
const DOCUMENT_VERSION: u32 = 1;

const SELECT_WORKERS: &str = "SELECT id, name, address, key_pin, version, details, paired_at, \
     connected_at, chosen_name FROM workers_paired ORDER BY paired_at, id";
const HAS_WORKERS: &str = "SELECT EXISTS (SELECT 1 FROM workers_paired)";
const INSERT_WORKER: &str = "INSERT INTO workers_paired \
     (id, name, address, key_pin, version, details, paired_at, updated_at, chosen_name) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)";
const UPDATE_HELLO: &str = "UPDATE workers_paired SET name = ?2, version = ?3, details = ?4, \
     connected_at = ?5, updated_at = ?5 WHERE id = ?1";
const UPDATE_CHOSEN_NAME: &str =
    "UPDATE workers_paired SET chosen_name = ?2, updated_at = ?3 WHERE id = ?1";
const DELETE_WORKER: &str = "DELETE FROM workers_paired WHERE id = ?1";

/// Every statement of this module, which its tests prepare against the
/// migrated schema (ADR-0009).
pub const STATEMENTS: &[&str] = &[
    SELECT_WORKERS,
    HAS_WORKERS,
    INSERT_WORKER,
    UPDATE_HELLO,
    UPDATE_CHOSEN_NAME,
    DELETE_WORKER,
];

/// Why the workers could not be read or saved.
#[derive(Debug, thiserror::Error)]
pub enum WorkersStoreError {
    /// A worker with the same key is already paired.
    #[error("a worker with this key is already paired")]
    KeyTaken,
    /// The worker is not paired.
    #[error("the worker is not paired")]
    WorkerNotFound,
    /// Stored data does not have the shape this version expects.
    #[error("stored data is not valid: {what}")]
    InvalidData {
        /// What is wrong.
        what: String,
    },
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl WorkersStoreError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::KeyTaken => "workers_store.key_taken",
            Self::WorkerNotFound => "workers_store.worker_not_found",
            Self::InvalidData { .. } => "workers_store.invalid_data",
            Self::Store(error) => error.code(),
        }
    }

    fn invalid(what: impl Into<String>) -> Self {
        Self::InvalidData { what: what.into() }
    }
}

impl From<TransactError<Self>> for WorkersStoreError {
    fn from(error: TransactError<Self>) -> Self {
        match error {
            TransactError::Rejected(error) => error,
            TransactError::Store(error) => Self::Store(error),
        }
    }
}

/// A worker to pair with.
#[derive(Debug, Clone)]
pub struct NewPairedWorker {
    /// The worker, as the master named it.
    pub id: WorkerId,
    /// The worker's name, as the worker gave it.
    pub name: String,
    /// The name that the owner gave the worker in Mannie, if any.
    pub chosen_name: Option<String>,
    /// The address the master connects to.
    pub address: String,
    /// The SHA-256 of the worker's public key, in hexadecimal.
    pub key_pin: String,
    /// What the worker said about itself.
    pub details: WorkerDetails,
}

/// A worker that the master paired with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairedWorker {
    /// The worker.
    pub id: WorkerId,
    /// The worker's name, as it gave it when it last connected.
    pub name: String,
    /// The name that the owner gave the worker in Mannie, if any.
    pub chosen_name: Option<String>,
    /// The address the master connects to.
    pub address: String,
    /// The SHA-256 of the worker's public key, in hexadecimal.
    pub key_pin: String,
    /// What the worker said about itself when it last connected.
    pub details: WorkerDetails,
    /// When the master paired with it.
    pub paired_at: UtcMillis,
    /// When the master last connected to it.
    pub connected_at: Option<UtcMillis>,
}

struct RawWorker {
    id: String,
    name: String,
    chosen_name: Option<String>,
    address: String,
    key_pin: String,
    version: i64,
    details: String,
    paired_at: i64,
    connected_at: Option<i64>,
}

impl RawWorker {
    fn into_worker(self) -> Result<PairedWorker, WorkersStoreError> {
        if self.version != i64::from(DOCUMENT_VERSION) {
            return Err(WorkersStoreError::invalid(format!(
                "worker {} has document version {}",
                self.id, self.version
            )));
        }
        let details = serde_json::from_str(&self.details)
            .map_err(|error| WorkersStoreError::invalid(format!("worker {}: {error}", self.id)))?;
        Ok(PairedWorker {
            id: WorkerId::parse(&self.id)
                .map_err(|error| WorkersStoreError::invalid(error.to_string()))?,
            name: self.name,
            chosen_name: self.chosen_name,
            address: self.address,
            key_pin: self.key_pin,
            details,
            paired_at: UtcMillis::from_millis(self.paired_at),
            connected_at: self.connected_at.map(UtcMillis::from_millis),
        })
    }
}

fn to_json<T: serde::Serialize>(value: &T) -> Result<String, WorkersStoreError> {
    serde_json::to_string(value).map_err(|error| {
        WorkersStoreError::invalid(format!("could not encode a document: {error}"))
    })
}

fn changed(rows: usize) -> Result<(), WorkersStoreError> {
    if rows == 0 {
        Err(WorkersStoreError::WorkerNotFound)
    } else {
        Ok(())
    }
}

/// The repository of the paired workers.
#[derive(Debug, Clone)]
pub struct WorkersStore {
    store: ModuleStore,
}

impl WorkersStore {
    /// The repository over the store of the plugin's module.
    pub fn new(store: ModuleStore) -> Self {
        Self { store }
    }

    /// The workers that the master paired with, in the order it paired.
    pub async fn paired_workers(&self) -> Result<Vec<PairedWorker>, WorkersStoreError> {
        let raw: Vec<RawWorker> = self
            .store
            .read(|connection| {
                let mut select = connection.prepare_cached(SELECT_WORKERS)?;
                let rows = select.query_map([], |row| {
                    Ok(RawWorker {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        address: row.get(2)?,
                        key_pin: row.get(3)?,
                        version: row.get(4)?,
                        details: row.get(5)?,
                        paired_at: row.get(6)?,
                        connected_at: row.get(7)?,
                        chosen_name: row.get(8)?,
                    })
                })?;
                rows.collect()
            })
            .await?;
        raw.into_iter().map(RawWorker::into_worker).collect()
    }

    /// Whether the master paired with any worker.
    pub async fn has_paired_workers(&self) -> Result<bool, WorkersStoreError> {
        let any = self
            .store
            .read(|connection| connection.query_row(HAS_WORKERS, [], |row| row.get(0)))
            .await?;
        Ok(any)
    }

    /// Stores a worker that the master paired with, with full durability.
    pub async fn add_paired_worker(
        &self,
        worker: NewPairedWorker,
    ) -> Result<(), WorkersStoreError> {
        let details = to_json(&worker.details)?;
        let now = self.store.clock().now().as_millis();
        self.store
            .transact(Durability::Full, move |transaction| {
                let inserted = transaction.prepare_cached(INSERT_WORKER)?.execute(params![
                    worker.id.as_str(),
                    worker.name,
                    worker.address,
                    worker.key_pin,
                    DOCUMENT_VERSION,
                    details,
                    now,
                    worker.chosen_name
                ]);
                match inserted {
                    Ok(_) => Ok(()),
                    Err(mannie_store::sql::Error::SqliteFailure(failure, _))
                        if failure.code == ErrorCode::ConstraintViolation =>
                    {
                        Err(TransactError::Rejected(WorkersStoreError::KeyTaken))
                    }
                    Err(error) => Err(error.into()),
                }
            })
            .await
            .map_err(WorkersStoreError::from)
    }

    /// Stores what a worker said about itself when the master connected.
    pub async fn update_worker_hello(
        &self,
        worker_id: &WorkerId,
        name: &str,
        details: &WorkerDetails,
    ) -> Result<(), WorkersStoreError> {
        let id = worker_id.as_str().to_owned();
        let name = name.to_owned();
        let details = to_json(details)?;
        let now = self.store.clock().now().as_millis();
        let rows = self
            .store
            .write(Durability::Normal, move |transaction| {
                transaction.prepare_cached(UPDATE_HELLO)?.execute(params![
                    id,
                    name,
                    DOCUMENT_VERSION,
                    details,
                    now
                ])
            })
            .await?;
        changed(rows)
    }

    /// Sets the name that the owner gave a worker in Mannie, or with `None`
    /// goes back to the name in its configuration.
    pub async fn set_worker_chosen_name(
        &self,
        worker_id: &WorkerId,
        chosen_name: Option<&str>,
    ) -> Result<(), WorkersStoreError> {
        let id = worker_id.as_str().to_owned();
        let chosen_name = chosen_name.map(str::to_owned);
        let now = self.store.clock().now().as_millis();
        let rows = self
            .store
            .write(Durability::Full, move |transaction| {
                transaction
                    .prepare_cached(UPDATE_CHOSEN_NAME)?
                    .execute(params![id, chosen_name, now])
            })
            .await?;
        changed(rows)
    }

    /// Forgets a worker. Returns whether it was paired.
    pub async fn remove_paired_worker(
        &self,
        worker_id: &WorkerId,
    ) -> Result<bool, WorkersStoreError> {
        let id = worker_id.as_str().to_owned();
        let rows = self
            .store
            .write(Durability::Full, move |transaction| {
                transaction.prepare_cached(DELETE_WORKER)?.execute([id])
            })
            .await?;
        Ok(rows > 0)
    }
}

#[cfg(test)]
mod tests;
