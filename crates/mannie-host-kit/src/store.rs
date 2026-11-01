//! The store as a program opens it, and the engine of the owner's SQLite
//! files with its registry and its journal of owner operations, which a
//! worker uses for the databases on its host (ADR-0056, decision 1).

pub use mannie_store::{RuntimeStoreError, Store, StoreError, StoreOptions, dbfiles};
