//! Mannie's store: one SQLite database per machine (ADR-0009).
//!
//! - One writer thread owns the only read-write connection, and every write is
//!   a short transaction on it. A small pool of read-only connections serves
//!   reads in parallel, which WAL mode allows.
//! - Each module owns its tables and their forward-only migrations, which are
//!   embedded in the binary; a composition root adds the modules of its
//!   plugins (ADR-0030). Before migrating an existing database, the store
//!   copies it; a database in which a module has a newer schema than the
//!   binary knows is refused and left untouched.
//! - The core's other crates use the repositories in this crate and never see
//!   SQLite. A plugin's code reaches its own tables through the SDK's service
//!   `store`, as statements that its module lists (ADR-0034, decision 7),
//!   which [`service_providers`] binds to the plugin.

mod error;
mod exports;
mod legacy;
mod plugin_store;
mod provider;
mod readers;
mod runtime;
mod schema;
mod settings;
mod store;
mod tools;
#[cfg(feature = "validation")]
pub mod validation;
mod writer;

pub use error::{DatabaseError, ModuleProblem, StoreError};
pub use plugin_store::{PluginModule, service_providers};
pub use provider::{ProfileFields, ProfileStoreError, StoredProfile};
pub use runtime::{
    AddedProject, Applied, Change, ConversationList, EventsAfter, ExecutorTaskProfile, KeptRun,
    ModelImage, ModelPart, ModelTurn, NewArchive, NewCheckpoint, NewConversation, NewExecutorTask,
    NewImage, NewMention, NewPart, NewRun, NewTask, OutboxEntry, ProjectDeletion, ProviderItem,
    Recovered, RuntimeStoreError, StoredArchive, StoredCheckpoint, StoredChild, StoredImage,
    StoredTask, TaskEnding, TaskExecution, TaskOrigin, UncertainCall,
};
pub use settings::SettingsStoreError;
pub use store::{Store, StoreOptions};
pub use tools::ToolsStoreError;

/// How durable a write is once its future resolves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability {
    /// Committed with `synchronous=FULL`: the write survives a crash or a
    /// power loss. Journal entries and state changes use it before the side
    /// effect they record.
    Full,
    /// Committed with `synchronous=NORMAL`: the write survives a crash of
    /// Mannie, and a power loss may lose the last commits but never corrupts
    /// the database. Batches of streamed deltas use it.
    Normal,
}

/// Every SQL statement of every repository, so a test can prepare each one
/// against the migrated schema (ADR-0009).
#[cfg(test)]
pub(crate) fn all_statements() -> impl Iterator<Item = &'static str> {
    settings::STATEMENTS
        .iter()
        .chain(exports::STATEMENTS)
        .chain(provider::STATEMENTS)
        .chain(tools::STATEMENTS)
        .copied()
        .chain(runtime::statements())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use mannie_types::clock::SystemClock;

    use crate::{Store, StoreOptions};

    #[tokio::test]
    async fn every_statement_prepares_against_the_migrated_schema() {
        let directory = tempfile::tempdir().unwrap();
        let options = StoreOptions::new(directory.path().join("backups"), Arc::new(SystemClock));
        let store = Store::open(&directory.path().join("mannie.sqlite"), &options).unwrap();

        let statements: Vec<&'static str> = super::all_statements().collect();
        assert_ne!(statements.len(), 0);
        let failures = store
            .read(move |connection| {
                Ok(statements
                    .into_iter()
                    .filter_map(|sql| {
                        connection
                            .prepare(sql)
                            .err()
                            .map(|error| format!("{sql}: {error}"))
                    })
                    .collect::<Vec<_>>())
            })
            .await
            .unwrap();

        assert!(failures.is_empty(), "{failures:#?}");
    }
}
