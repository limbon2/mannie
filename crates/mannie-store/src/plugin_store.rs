//! The modules of plugins (ADR-0034, decision 7; ADR-0030, decision 9): a
//! plugin's tables, which its code reaches as statements that are data
//! through the SDK's service `store`.
//!
//! The composition root hands the store each plugin's module with the
//! plugin's prefix. The store checks the module before it touches the
//! database: its name must be the prefix, its versions must count from 1,
//! and its numbers of the list before M5 must be the ones that the frozen
//! record gives it. Its migrations then run with the core's. After the
//! migrations, the store prepares every statement that the module lists. A
//! statement that does not prepare leaves the module migrated, and its plugin
//! cannot reach the store, so its start fails and the rest of Mannie keeps
//! working.
//!
//! A plugin's statement is always prepared afresh, never through the
//! statement cache that the core's repositories share, and it prepares and
//! steps under the SQLite authorizer, which confines it to the plugin's
//! tables, as the plugin's migrations are (ADR-0034, decision 7). No plugin
//! code runs on the writer's thread: a batch is store code that runs the
//! listed statements in one transaction.

mod authorizer;
mod constraints;
#[cfg(test)]
mod tests;

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::host::{BindRefusal, PluginBinding, ServiceProvider};
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::service::{BoundService, Service};
use mannie_plugin_sdk::services::store as sdk;
use rusqlite::types::Value;
use rusqlite::{Connection, params_from_iter};

pub(crate) use self::authorizer::run_plugin_migration;
use self::authorizer::{Policy, confined};
use crate::error::{ModuleProblem, StoreError};
use crate::legacy::LEGACY_VERSIONS;
use crate::schema::{Migration, StoreModule};
use crate::writer::TransactError;
use crate::{Durability, Store};

/// A plugin's module, as the composition root hands it to the store: the
/// module that the plugin's code declares, with the prefix of the plugin
/// that it belongs to.
#[derive(Debug, Clone)]
pub struct PluginModule {
    prefix: String,
    module: sdk::StoreModule,
}

impl PluginModule {
    /// The module of the plugin `plugin`.
    pub fn new(plugin: &PluginId, module: sdk::StoreModule) -> Self {
        Self {
            prefix: plugin.prefix(),
            module,
        }
    }
}

/// Checks a plugin's module and turns it into a module of the schema. A
/// module that would corrupt the store's records or skip a migration is
/// refused before the database is touched.
pub(crate) fn schema_module(plugin: &PluginModule) -> Result<StoreModule, StoreError> {
    let module = &plugin.module;
    let refused = |problem| StoreError::ModuleRefused {
        module: module.name.to_owned(),
        problem,
    };
    if module.name != plugin.prefix {
        return Err(refused(ModuleProblem::NotThePrefix {
            prefix: plugin.prefix.clone(),
        }));
    }
    for (index, migration) in module.migrations.iter().enumerate() {
        let position = u32::try_from(index + 1).unwrap_or(u32::MAX);
        let version = migration.version();
        if version != position {
            return Err(refused(ModuleProblem::VersionOutOfOrder {
                position,
                version,
            }));
        }
        if migration.sql().trim().is_empty() {
            return Err(refused(ModuleProblem::EmptyMigration { version }));
        }
        if let Some(legacy_version) = migration.legacy_version() {
            let recorded = LEGACY_VERSIONS.iter().any(|recorded| {
                recorded.module == module.name
                    && recorded.version == version
                    && recorded.legacy_version == legacy_version
            });
            if !recorded {
                return Err(refused(ModuleProblem::LegacyVersionNotRecorded {
                    version,
                    legacy_version,
                }));
            }
        }
    }
    for recorded in LEGACY_VERSIONS
        .iter()
        .filter(|recorded| recorded.module == module.name)
    {
        let declared = module.migrations.iter().any(|migration| {
            migration.version() == recorded.version
                && migration.legacy_version() == Some(recorded.legacy_version)
        });
        if !declared {
            return Err(refused(ModuleProblem::LegacyVersionMissing {
                version: recorded.version,
                legacy_version: recorded.legacy_version,
            }));
        }
    }
    let migrations = module
        .migrations
        .iter()
        .map(|migration| Migration {
            version: migration.version(),
            legacy_version: migration.legacy_version(),
            description: migration.description(),
            sql: migration.sql(),
        })
        .collect();
    Ok(StoreModule {
        name: module.name,
        migrations: Cow::Owned(migrations),
        confined: true,
    })
}

/// A plugin's module as the store opened it: its prefix, and its listed
/// statements, or `None` when one of them did not prepare.
#[derive(Debug)]
pub(crate) struct RegisteredModule {
    prefix: String,
    statements: Option<Arc<ListedStatements>>,
}

/// The statements that a module lists, by their text.
#[derive(Debug)]
struct ListedStatements {
    module: &'static str,
    by_text: HashMap<&'static str, Listed>,
}

/// What the store learned of a listed statement when it prepared it.
#[derive(Debug, Clone, Copy)]
struct Listed {
    /// Whether the statement only reads, so a query may run it.
    read_only: bool,
}

impl ListedStatements {
    /// Refuses a query of text that the module does not list, or of a
    /// statement that would write.
    fn check_query(&self, statement: &sdk::Statement) -> Result<(), sdk::StoreError> {
        let listed = self.listed(statement)?;
        if listed.read_only {
            Ok(())
        } else {
            tracing::error!(
                module = self.module,
                "a plugin asked to query with a statement that writes"
            );
            Err(sdk::StoreError::NotPermitted)
        }
    }

    /// Refuses a batch in which any statement is not one that the module
    /// lists, before anything of it runs.
    fn check_batch(&self, batch: &sdk::WriteBatch) -> Result<(), sdk::StoreError> {
        for (statement, _expect) in batch.statements() {
            self.listed(statement)?;
        }
        Ok(())
    }

    fn listed(&self, statement: &sdk::Statement) -> Result<Listed, sdk::StoreError> {
        self.by_text.get(statement.sql()).copied().ok_or_else(|| {
            tracing::error!(
                module = self.module,
                "a plugin asked for a statement that its module does not list"
            );
            sdk::StoreError::UnknownStatement
        })
    }
}

/// Prepares every statement that a plugin's module lists, on the store's
/// read-write connection after the migrations. A statement that does not
/// prepare is logged with SQLite's reason and leaves the plugin without its
/// store.
pub(crate) fn register(connection: &Connection, plugin: &PluginModule) -> RegisteredModule {
    let module = plugin.module.name;
    let mut by_text = HashMap::new();
    let mut prepared = true;
    for (index, sql) in plugin.module.statements.iter().enumerate() {
        if sql.trim().is_empty() {
            tracing::error!(
                module,
                statement = index,
                "a statement that a plugin's module lists is empty"
            );
            prepared = false;
            continue;
        }
        let checked = confined(connection, &Policy::statement(module), |connection| {
            prepare_plugin_statement(connection, sql).map(|statement| statement.readonly())
        });
        match checked {
            Ok(read_only) => {
                by_text.insert(*sql, Listed { read_only });
            }
            Err(error) => {
                tracing::error!(module, statement = index, %error, "a statement that a plugin's module lists does not prepare");
                prepared = false;
            }
        }
    }
    RegisteredModule {
        prefix: plugin.prefix.clone(),
        statements: prepared.then(|| Arc::new(ListedStatements { module, by_text })),
    }
}

/// Prepares a plugin's statement. Always afresh, never through the statement
/// cache that the core's repositories share, so that each plugin statement is
/// prepared under the plugin's own rules (ADR-0034, decision 7). The caller
/// has the authorizer installed.
fn prepare_plugin_statement<'connection>(
    connection: &'connection Connection,
    sql: &str,
) -> rusqlite::Result<rusqlite::Statement<'connection>> {
    connection.prepare(sql)
}

fn to_value(value: &sdk::SqlValue) -> Value {
    match value {
        sdk::SqlValue::Null => Value::Null,
        sdk::SqlValue::Integer(integer) => Value::Integer(*integer),
        sdk::SqlValue::Real(real) => Value::Real(*real),
        sdk::SqlValue::Text(text) => Value::Text(text.clone()),
        sdk::SqlValue::Blob(bytes) => Value::Blob(bytes.clone()),
    }
}

fn from_value(value: Value) -> sdk::SqlValue {
    match value {
        Value::Null => sdk::SqlValue::Null,
        Value::Integer(integer) => sdk::SqlValue::Integer(integer),
        Value::Real(real) => sdk::SqlValue::Real(real),
        Value::Text(text) => sdk::SqlValue::Text(text),
        Value::Blob(bytes) => sdk::SqlValue::Blob(bytes),
    }
}

/// Runs a statement of the plugin's module `module` under its authorizer
/// and returns its rows.
fn run_query(
    connection: &Connection,
    module: &str,
    statement: &sdk::Statement,
) -> rusqlite::Result<Vec<sdk::Row>> {
    confined(connection, &Policy::statement(module), |connection| {
        query_rows(connection, statement)
    })
}

/// Runs a statement and returns its rows.
fn query_rows(
    connection: &Connection,
    statement: &sdk::Statement,
) -> rusqlite::Result<Vec<sdk::Row>> {
    let mut prepared = prepare_plugin_statement(connection, statement.sql())?;
    let columns = prepared.column_count();
    let mut rows = prepared.query(params_from_iter(statement.params().iter().map(to_value)))?;
    let mut found = Vec::new();
    while let Some(row) = rows.next()? {
        let mut values = Vec::with_capacity(columns);
        for column in 0..columns {
            values.push(from_value(row.get::<_, Value>(column)?));
        }
        found.push(sdk::Row::new(values));
    }
    Ok(found)
}

/// Runs the statements of a batch of the plugin's module `module` in the
/// writer's transaction, each under the module's authorizer, and refuses
/// with the statement's position when it changed another number of rows
/// than it expected, which rolls the batch back.
fn run_batch(
    connection: &Connection,
    module: &str,
    statements: &[(sdk::Statement, sdk::Expect)],
) -> Result<Vec<u64>, TransactError<usize>> {
    let policy = Policy::statement(module);
    let mut counts = Vec::with_capacity(statements.len());
    for (index, (statement, expect)) in statements.iter().enumerate() {
        let changed = confined(connection, &policy, |connection| {
            let mut prepared = prepare_plugin_statement(connection, statement.sql())?;
            prepared.execute(params_from_iter(statement.params().iter().map(to_value)))
        })?;
        let changed = u64::try_from(changed).unwrap_or(u64::MAX);
        if let sdk::Expect::Changes(expected) = expect
            && changed != *expected
        {
            return Err(TransactError::Rejected(index));
        }
        counts.push(changed);
    }
    Ok(counts)
}

/// The SDK's error for a failure of the store. A failed constraint that the
/// batch mapped gets the plugin's code; every other failure is logged with
/// SQLite's message, which names tables and columns and never a bound value.
fn failure(
    module: &'static str,
    error: &StoreError,
    constraints: &[(sdk::Constraint, &'static str)],
) -> sdk::StoreError {
    match error {
        StoreError::WriterStopped | StoreError::ReadersStopped => sdk::StoreError::Unavailable,
        StoreError::Query { source } => {
            if let Some(failed) = constraints::failed(&source.0) {
                let mapped = constraints
                    .iter()
                    .find(|(constraint, _code)| failed.is(constraint));
                if let Some((_constraint, code)) = mapped {
                    return sdk::StoreError::Constraint { code };
                }
            }
            tracing::error!(module, error.code = error.code(), %source, "a statement of a plugin's module failed");
            sdk::StoreError::Failed
        }
        other => {
            tracing::error!(
                module,
                error.code = other.code(),
                "a plugin's module could not use the store"
            );
            sdk::StoreError::Failed
        }
    }
}

/// A plugin's module of the store, bound to the plugin.
#[derive(Debug, Clone)]
struct PluginStore {
    store: Store,
    statements: Arc<ListedStatements>,
}

impl sdk::ModuleQueries for PluginStore {
    fn query(
        &self,
        statement: sdk::Statement,
    ) -> BoxFuture<'static, Result<Vec<sdk::Row>, sdk::StoreError>> {
        let checked = self.statements.check_query(&statement);
        let store = self.store.clone();
        let module = self.statements.module;
        Box::pin(async move {
            checked?;
            store
                .read(move |connection| run_query(connection, module, &statement))
                .await
                .map_err(|error| failure(module, &error, &[]))
        })
    }
}

impl sdk::ModuleStoreApi for PluginStore {
    fn write(
        &self,
        batch: sdk::WriteBatch,
        durability: sdk::Durability,
    ) -> BoxFuture<'static, Result<Vec<u64>, sdk::StoreError>> {
        let checked = self.statements.check_batch(&batch);
        let store = self.store.clone();
        let module = self.statements.module;
        Box::pin(async move {
            checked?;
            if batch.is_empty() {
                return Ok(Vec::new());
            }
            let erases = durability == sdk::Durability::Erase;
            let durability = match durability {
                sdk::Durability::Normal => Durability::Normal,
                sdk::Durability::Full | sdk::Durability::Erase => Durability::Full,
            };
            let constraints = batch.constraints().to_vec();
            let statements = batch.statements().to_vec();
            // The mapping happens after the transaction, so a deferred
            // foreign key that fails when the batch commits maps the same.
            let changes = match store
                .transact(durability, move |transaction| {
                    run_batch(transaction, module, &statements)
                })
                .await
            {
                Ok(changes) => changes,
                Err(TransactError::Rejected(index)) => {
                    return Err(sdk::StoreError::Expectation { index });
                }
                Err(TransactError::Store(error)) => {
                    return Err(failure(module, &error, &constraints));
                }
            };
            // The writer's `secure_delete` overwrote the freed pages, and
            // emptying the write-ahead log removes their old versions from it,
            // as deleting a conversation does (security architecture, local
            // data).
            if erases && let Err(error) = store.flush().await {
                tracing::warn!(module, error = %error, "the log of a plugin's erasing write could not be emptied");
            }
            Ok(changes)
        })
    }
}

impl Store {
    /// The module of the plugin `plugin`, as the service `store` binds it.
    /// The composition root and the host reach it through the store's
    /// provider; tests reach it directly.
    pub fn plugin_store(
        &self,
        plugin: &PluginId,
    ) -> Result<Arc<dyn sdk::ModuleStoreApi>, StoreError> {
        let prefix = plugin.prefix();
        let registered = self
            .plugin_modules()
            .iter()
            .find(|registered| registered.prefix == prefix)
            .ok_or_else(|| StoreError::ModuleNotRegistered {
                module: prefix.clone(),
            })?;
        let statements = registered
            .statements
            .clone()
            .ok_or(StoreError::StatementsRefused { module: prefix })?;
        Ok(Arc::new(PluginStore {
            store: self.clone(),
            statements,
        }))
    }

    /// Runs SQL that no module lists, such as the rows that an upgrade test
    /// seeds into a plugin's tables as an older version wrote them.
    #[cfg(any(test, feature = "testing"))]
    pub async fn execute_unlisted(&self, sql: &'static str) -> Result<(), StoreError> {
        self.write(Durability::Full, move |transaction| {
            transaction.execute_batch(sql)
        })
        .await
    }

    /// Queries with SQL that no module lists, such as what an upgrade test
    /// checks in a plugin's tables.
    #[cfg(any(test, feature = "testing"))]
    pub async fn query_unlisted(&self, sql: &'static str) -> Result<Vec<sdk::Row>, StoreError> {
        self.read(move |connection| query_rows(connection, &sdk::Statement::new(sql)))
            .await
    }
}

/// The store's provider of the SDK's service `store` (ADR-0034, decision 5),
/// which binds a plugin to its own module.
#[derive(Debug)]
struct StoreProvider {
    store: Store,
}

impl ServiceProvider for StoreProvider {
    fn name(&self) -> &'static str {
        sdk::Store::NAME
    }

    fn major(&self) -> u16 {
        sdk::Store::MAJOR
    }

    fn bind(&self, binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        self.store
            .plugin_store(binding.id)
            .map(BoundService::new::<sdk::Store>)
            .map_err(|error| BindRefusal::new(sdk::Store::NAME, error.to_string()))
    }
}

/// The services that the store provides, for the composition root to hand
/// the plugin host.
pub fn service_providers(store: &Store) -> Vec<Arc<dyn ServiceProvider>> {
    vec![Arc::new(StoreProvider {
        store: store.clone(),
    })]
}
