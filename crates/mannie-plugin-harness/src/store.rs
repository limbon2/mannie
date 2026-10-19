//! The store's parts that plugin tests read and seed: the store with the
//! plugins' modules, the records of conversations, runs, parts and tasks, the
//! outbox, and the store's providers of services.

pub use mannie_store::{
    Change, Durability, NewConversation, NewExecutorTask, NewPart, NewRun, OutboxEntry,
    PluginModule, Store, StoreOptions, StoredTask, TaskOrigin, service_providers,
};
