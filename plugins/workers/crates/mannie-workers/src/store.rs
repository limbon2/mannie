//! The Workers plugin's tables (plugins/workers/docs/pairing-and-link.md,
//! data; ADR-0030, decision 9): the workers that the master paired with.
//!
//! The module's migrations run at startup with the core's, whether or not
//! Workers is on, so the schema never depends on the plugin's state. Before
//! M5 they were migrations 19, 20 and 22 of the store's single list, whose
//! numbers carry over the version of an older database (ADR-0030,
//! decision 5). The plugin's code reaches its tables through the SDK's
//! service `store`, as the statements that the module lists (ADR-0034,
//! decision 7): the store prepares each one when it registers the module, so
//! a statement that no longer fits the schema fails Workers' start.

use std::sync::Arc;

use mannie_link::host::WorkerDetails;
use mannie_plugin_sdk::contracts::{Clock, UtcMillis, WorkerId};
use mannie_plugin_sdk::services::store::{
    Constraint, Durability, Expect, Migration, ModuleQueries, ModuleStoreApi, Row, Statement,
    StoreError, StoreModule, WriteBatch,
};

const MIGRATIONS: &[Migration] = &[
    Migration::legacy(
        1,
        19,
        "create the paired workers table",
        include_str!("store/migrations/0019_create_paired_workers.sql"),
    ),
    Migration::legacy(
        2,
        20,
        "create the table of deletions that wait for a worker",
        include_str!("store/migrations/0020_create_worker_deletions.sql"),
    ),
    Migration::legacy(
        3,
        22,
        "add the name that the owner gives a worker",
        include_str!("store/migrations/0022_add_worker_chosen_names.sql"),
    ),
    Migration::new(
        4,
        "move the deletions that wait for a worker into the core's outbox",
        include_str!("store/migrations/0028_move_worker_deletions_to_the_outbox.sql"),
    ),
];

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

/// Every statement of this module, which the store prepares against the
/// migrated schema when it registers the module (ADR-0009).
const STATEMENTS: &[&str] = &[
    SELECT_WORKERS,
    HAS_WORKERS,
    INSERT_WORKER,
    UPDATE_HELLO,
    UPDATE_CHOSEN_NAME,
    DELETE_WORKER,
];

/// The plugin's module of the store: the tables that `workers_` prefixes.
pub const WORKERS_MODULE: StoreModule = StoreModule::new("workers", MIGRATIONS, STATEMENTS);

/// The code that a batch gives a pin that another paired worker has.
const KEY_TAKEN: &str = "key_taken";

/// The unique pin of a paired worker.
const KEY_PIN: Constraint = Constraint::Key {
    table: "workers_paired",
    columns: &["key_pin"],
};

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
    /// A row of `SELECT_WORKERS`.
    fn from_row(row: &Row) -> Result<Self, StoreError> {
        Ok(Self {
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
    }

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

/// Whether the master paired with any worker, which Workers' default reads
/// before the plugin starts (ADR-0031, decision 10).
pub async fn has_paired_workers(queries: &dyn ModuleQueries) -> Result<bool, WorkersStoreError> {
    let rows = queries.query(Statement::new(HAS_WORKERS)).await?;
    let Some(row) = rows.first() else {
        return Err(WorkersStoreError::invalid(
            "the question whether a worker is paired returned no row",
        ));
    };
    Ok(row.get(0)?)
}

/// The repository of the paired workers.
#[derive(Debug, Clone)]
pub struct WorkersStore {
    store: Arc<dyn ModuleStoreApi>,
    clock: Arc<dyn Clock>,
}

impl WorkersStore {
    /// The repository over the plugin's module of the store, which stamps
    /// its rows with `clock`.
    pub fn new(store: Arc<dyn ModuleStoreApi>, clock: Arc<dyn Clock>) -> Self {
        Self { store, clock }
    }

    /// The workers that the master paired with, in the order it paired.
    pub async fn paired_workers(&self) -> Result<Vec<PairedWorker>, WorkersStoreError> {
        let rows = self.store.query(Statement::new(SELECT_WORKERS)).await?;
        rows.iter()
            .map(|row| RawWorker::from_row(row)?.into_worker())
            .collect()
    }

    /// Stores a worker that the master paired with, with full durability.
    pub async fn add_paired_worker(
        &self,
        worker: NewPairedWorker,
    ) -> Result<(), WorkersStoreError> {
        let details = to_json(&worker.details)?;
        let now = self.clock.now().as_millis();
        let insert = Statement::new(INSERT_WORKER)
            .bind(worker.id.as_str())
            .bind(worker.name)
            .bind(worker.address)
            .bind(worker.key_pin)
            .bind(DOCUMENT_VERSION)
            .bind(details)
            .bind(now)
            .bind(worker.chosen_name);
        let batch = WriteBatch::new()
            .push(insert, Expect::Changes(1))
            .on_constraint(KEY_PIN, KEY_TAKEN);
        match self.store.write(batch, Durability::Full).await {
            Ok(_changes) => Ok(()),
            Err(StoreError::Constraint { code: KEY_TAKEN }) => Err(WorkersStoreError::KeyTaken),
            Err(error) => Err(error.into()),
        }
    }

    /// Stores what a worker said about itself when the master connected.
    pub async fn update_worker_hello(
        &self,
        worker_id: &WorkerId,
        name: &str,
        details: &WorkerDetails,
    ) -> Result<(), WorkersStoreError> {
        let details = to_json(details)?;
        let now = self.clock.now().as_millis();
        let update = Statement::new(UPDATE_HELLO)
            .bind(worker_id.as_str())
            .bind(name)
            .bind(DOCUMENT_VERSION)
            .bind(details)
            .bind(now);
        self.change_worker(update, Durability::Normal).await
    }

    /// Sets the name that the owner gave a worker in Mannie, or with `None`
    /// goes back to the name in its configuration.
    pub async fn set_worker_chosen_name(
        &self,
        worker_id: &WorkerId,
        chosen_name: Option<&str>,
    ) -> Result<(), WorkersStoreError> {
        let now = self.clock.now().as_millis();
        let update = Statement::new(UPDATE_CHOSEN_NAME)
            .bind(worker_id.as_str())
            .bind(chosen_name)
            .bind(now);
        self.change_worker(update, Durability::Full).await
    }

    /// Forgets a worker. Returns whether it was paired.
    pub async fn remove_paired_worker(
        &self,
        worker_id: &WorkerId,
    ) -> Result<bool, WorkersStoreError> {
        let delete = Statement::new(DELETE_WORKER).bind(worker_id.as_str());
        let changes = self
            .store
            .write(
                WriteBatch::new().push(delete, Expect::Any),
                Durability::Full,
            )
            .await?;
        Ok(changes.first().is_some_and(|changed| *changed > 0))
    }

    /// Runs a statement that changes one paired worker's row, and refuses
    /// with `WorkerNotFound` when the worker is not paired.
    async fn change_worker(
        &self,
        statement: Statement,
        durability: Durability,
    ) -> Result<(), WorkersStoreError> {
        let batch = WriteBatch::new().push(statement, Expect::Changes(1));
        match self.store.write(batch, durability).await {
            Ok(_changes) => Ok(()),
            Err(StoreError::Expectation { index: 0 }) => Err(WorkersStoreError::WorkerNotFound),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests;
