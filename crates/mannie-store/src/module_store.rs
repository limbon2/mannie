//! The store as the code of one module uses it (ADR-0030, decision 9).

use mannie_types::clock::Clock;
use rusqlite::{Connection, Transaction};

use crate::error::StoreError;
use crate::schema::StoreModule;
use crate::writer::TransactError;
use crate::{Durability, Store};

/// The writer and readers of the database, for the code of one module, which
/// reads and writes the tables that the module's name prefixes.
///
/// The core's repositories live in this crate. A plugin's code holds one of
/// these for the module that its composition root registered, so SQLite stays
/// a dependency of this crate alone and every write still goes through the
/// one writer.
#[derive(Debug, Clone)]
pub struct ModuleStore {
    store: Store,
    module: &'static str,
}

impl Store {
    /// The store for the code of `module`, which must be one of the modules
    /// that this store opened with.
    pub fn module(&self, module: &StoreModule) -> Result<ModuleStore, StoreError> {
        if self.modules().contains(&module.name) {
            Ok(ModuleStore {
                store: self.clone(),
                module: module.name,
            })
        } else {
            Err(StoreError::ModuleNotRegistered {
                module: module.name,
            })
        }
    }
}

impl ModuleStore {
    /// The module's name.
    pub fn name(&self) -> &'static str {
        self.module
    }

    /// The store's clock.
    pub fn clock(&self) -> &dyn Clock {
        self.store.clock()
    }

    /// Runs `work` in a transaction on the writer thread. The future resolves
    /// once the transaction has committed with the given durability.
    pub async fn write<T, F>(&self, durability: Durability, work: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> rusqlite::Result<T> + Send + 'static,
    {
        self.store.write(durability, work).await
    }

    /// Runs `work` in a transaction that it may refuse: when `work` returns an
    /// error, nothing is written and the error is returned.
    pub async fn transact<T, E, F>(
        &self,
        durability: Durability,
        work: F,
    ) -> Result<T, TransactError<E>>
    where
        T: Send + 'static,
        E: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> Result<T, TransactError<E>> + Send + 'static,
    {
        self.store.transact(durability, work).await
    }

    /// Runs `work` on a read-only connection.
    pub async fn read<T, F>(&self, work: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        self.store.read(work).await
    }

    /// Prepares each statement against the migrated schema and returns those
    /// that do not prepare, with SQLite's reason. Each module's tests make
    /// this check for every statement of the module (ADR-0009).
    pub async fn unpreparable(
        &self,
        statements: &[&'static str],
    ) -> Result<Vec<String>, StoreError> {
        let statements = statements.to_vec();
        self.read(move |connection| {
            Ok(statements
                .into_iter()
                .filter_map(|sql| {
                    connection
                        .prepare(sql)
                        .err()
                        .map(|error| format!("{sql}: {error}"))
                })
                .collect())
        })
        .await
    }
}
