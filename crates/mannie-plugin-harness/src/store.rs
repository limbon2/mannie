//! The store's parts that plugin tests read and seed: the store with the
//! plugins' modules, the records of conversations, runs, parts and tasks, the
//! outbox, the store's providers of services, and the owner's SQLite files:
//! the files that tests of the service `sqlite-files` register and the
//! states of the owner operations on them (ADR-0056).

pub use mannie_store::{
    Change, Durability, NewConversation, NewExecutorTask, NewPart, NewRun, OutboxEntry,
    PluginModule, Store, StoreOptions, StoredTask, TaskOrigin, service_providers,
};

/// The owner's SQLite files: a file with tables and rows for a test, and the
/// states of the owner operations that the journal records.
pub mod dbfiles {
    pub use mannie_store::dbfiles::{OperationState, create_for_tests};
}
