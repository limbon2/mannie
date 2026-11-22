//! SQLite persistence module for the Network Client plugin (ADR-0075;
//! ADR-0009; ADR-0048).
//!
//! All `network_*` tables store non-secret metadata, secret flags and chunk
//! counts only. Secret values and cookie values live exclusively in the OS
//! credential store via [`crate::secrets::Vault`]. Every deletion uses
//! [`Durability::Erase`] so deleted data leaves the database and write-ahead
//! log immediately.

use std::sync::Arc;

use mannie_plugin_sdk::contracts::UtcMillis;
use mannie_plugin_sdk::services::store::{
    Constraint, Durability, Expect, Migration, ModuleStoreApi, Row, Statement, StoreError,
    StoreModule, WriteBatch,
};
use serde::{Deserialize, Serialize};

use crate::model::{
    AuthConfig, BodyConfig, HttpMethod, KeyValueEntry, NetworkError, Protocol, VariableEntry,
};

/// The SQLite table prefix and store module name of this plugin.
pub(crate) const PREFIX: &str = "network";

const MIGRATIONS: &[Migration] = &[
    Migration::new(
        1,
        "create the tables of collections, folders, requests, environments, cookies and global variables",
        "CREATE TABLE network_collections (
        id TEXT PRIMARY KEY NOT NULL,
        name TEXT NOT NULL UNIQUE COLLATE NOCASE,
        description TEXT NOT NULL,
        protocol TEXT NOT NULL CHECK (protocol IN ('http')),
        base_url TEXT NOT NULL,
        enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
        active_environment_id TEXT,
        auto_cookies INTEGER NOT NULL CHECK (auto_cookies IN (0, 1)),
        document TEXT NOT NULL CHECK (json_valid(document)),
        secret_parts INTEGER NOT NULL DEFAULT 0 CHECK (secret_parts >= 0 AND secret_parts <= 1000),
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE network_folders (
        id TEXT PRIMARY KEY NOT NULL,
        collection_id TEXT NOT NULL REFERENCES network_collections (id) ON DELETE CASCADE,
        parent_id TEXT REFERENCES network_folders (id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        sort_order INTEGER NOT NULL DEFAULT 0 CHECK (sort_order >= 0),
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX network_folders_by_collection ON network_folders (collection_id, sort_order, id);
    CREATE TABLE network_requests (
        id TEXT PRIMARY KEY NOT NULL,
        collection_id TEXT NOT NULL REFERENCES network_collections (id) ON DELETE CASCADE,
        folder_id TEXT REFERENCES network_folders (id) ON DELETE SET NULL,
        name TEXT NOT NULL,
        description TEXT NOT NULL,
        protocol TEXT NOT NULL CHECK (protocol IN ('http')),
        method TEXT NOT NULL,
        url_template TEXT NOT NULL,
        timeout_seconds INTEGER NOT NULL CHECK (timeout_seconds >= 1 AND timeout_seconds <= 300),
        use_cookies INTEGER NOT NULL CHECK (use_cookies IN (0, 1)),
        sort_order INTEGER NOT NULL DEFAULT 0 CHECK (sort_order >= 0),
        document TEXT NOT NULL CHECK (json_valid(document)),
        secret_parts INTEGER NOT NULL DEFAULT 0 CHECK (secret_parts >= 0 AND secret_parts <= 1000),
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX network_requests_by_collection ON network_requests (collection_id, sort_order, id);
    CREATE TABLE network_environments (
        id TEXT PRIMARY KEY NOT NULL,
        collection_id TEXT NOT NULL REFERENCES network_collections (id) ON DELETE CASCADE,
        name TEXT NOT NULL,
        document TEXT NOT NULL CHECK (json_valid(document)),
        secret_parts INTEGER NOT NULL DEFAULT 0 CHECK (secret_parts >= 0 AND secret_parts <= 1000),
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        UNIQUE (collection_id, name COLLATE NOCASE)
    ) STRICT;
    CREATE INDEX network_environments_by_collection ON network_environments (collection_id, name);
    CREATE TABLE network_cookies (
        id TEXT PRIMARY KEY NOT NULL,
        name TEXT NOT NULL,
        domain TEXT NOT NULL,
        path TEXT NOT NULL,
        secure INTEGER NOT NULL CHECK (secure IN (0, 1)),
        http_only INTEGER NOT NULL CHECK (http_only IN (0, 1)),
        expires_at INTEGER,
        enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
        secret_parts INTEGER NOT NULL DEFAULT 0 CHECK (secret_parts >= 0 AND secret_parts <= 1000),
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        UNIQUE (name, domain, path)
    ) STRICT;
    CREATE TABLE network_global_variables (
        id INTEGER PRIMARY KEY CHECK (id = 1),
        document TEXT NOT NULL CHECK (json_valid(document)),
        secret_parts INTEGER NOT NULL DEFAULT 0 CHECK (secret_parts >= 0 AND secret_parts <= 1000),
        updated_at INTEGER NOT NULL
    ) STRICT",
    ),
    Migration::new(
        2,
        "add host_only flag to cookies and atomic secret_slot references to secret-bearing tables",
        "ALTER TABLE network_cookies ADD COLUMN host_only INTEGER NOT NULL DEFAULT 0 CHECK (host_only IN (0, 1));
    ALTER TABLE network_collections ADD COLUMN secret_slot TEXT NOT NULL DEFAULT '';
    ALTER TABLE network_requests ADD COLUMN secret_slot TEXT NOT NULL DEFAULT '';
    ALTER TABLE network_environments ADD COLUMN secret_slot TEXT NOT NULL DEFAULT '';
    ALTER TABLE network_cookies ADD COLUMN secret_slot TEXT NOT NULL DEFAULT '';
    ALTER TABLE network_global_variables ADD COLUMN secret_slot TEXT NOT NULL DEFAULT ''",
    ),
];

const SELECT_COLLECTIONS: &str = "SELECT id, name, description, protocol, base_url, enabled, \
    active_environment_id, auto_cookies, document, secret_parts, secret_slot, created_at, updated_at \
    FROM network_collections ORDER BY created_at ASC, id ASC";

const INSERT_COLLECTION: &str = "INSERT INTO network_collections \
    (id, name, description, protocol, base_url, enabled, active_environment_id, auto_cookies, \
     document, secret_parts, secret_slot, created_at, updated_at) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)";

const UPDATE_COLLECTION: &str = "UPDATE network_collections \
    SET name = ?2, description = ?3, base_url = ?4, enabled = ?5, active_environment_id = ?6, \
        auto_cookies = ?7, document = ?8, secret_parts = ?9, secret_slot = ?10, updated_at = ?11 \
    WHERE id = ?1";

const SET_COLLECTION_ENABLED: &str =
    "UPDATE network_collections SET enabled = ?2, updated_at = ?3 WHERE id = ?1";

const SET_COLLECTION_ACTIVE_ENV: &str =
    "UPDATE network_collections SET active_environment_id = ?2, updated_at = ?3 WHERE id = ?1";

const DELETE_COLLECTION: &str = "DELETE FROM network_collections WHERE id = ?1";

const SELECT_FOLDERS: &str = "SELECT id, collection_id, parent_id, name, sort_order, created_at, \
    updated_at FROM network_folders ORDER BY sort_order ASC, created_at ASC, id ASC";

const INSERT_FOLDER: &str = "INSERT INTO network_folders \
    (id, collection_id, parent_id, name, sort_order, created_at, updated_at) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)";

const RENAME_FOLDER: &str = "UPDATE network_folders SET name = ?2, updated_at = ?3 WHERE id = ?1";

const DELETE_FOLDER: &str = "DELETE FROM network_folders WHERE id = ?1";

const SELECT_REQUESTS: &str = "SELECT id, collection_id, folder_id, name, description, protocol, \
    method, url_template, timeout_seconds, use_cookies, sort_order, document, secret_parts, \
    secret_slot, created_at, updated_at FROM network_requests ORDER BY sort_order ASC, created_at ASC, id ASC";

const INSERT_REQUEST: &str = "INSERT INTO network_requests \
    (id, collection_id, folder_id, name, description, protocol, method, url_template, \
     timeout_seconds, use_cookies, sort_order, document, secret_parts, secret_slot, created_at, updated_at) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)";

const UPDATE_REQUEST: &str = "UPDATE network_requests \
    SET folder_id = ?2, name = ?3, description = ?4, method = ?5, url_template = ?6, \
        timeout_seconds = ?7, use_cookies = ?8, document = ?9, secret_parts = ?10, \
        secret_slot = ?11, updated_at = ?12 \
    WHERE id = ?1";

const DELETE_REQUEST: &str = "DELETE FROM network_requests WHERE id = ?1";

const SELECT_ENVIRONMENTS: &str = "SELECT id, collection_id, name, document, secret_parts, \
    secret_slot, created_at, updated_at FROM network_environments ORDER BY name ASC, id ASC";

const INSERT_ENVIRONMENT: &str = "INSERT INTO network_environments \
    (id, collection_id, name, document, secret_parts, secret_slot, created_at, updated_at) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)";

const UPDATE_ENVIRONMENT: &str = "UPDATE network_environments \
    SET name = ?2, document = ?3, secret_parts = ?4, secret_slot = ?5, updated_at = ?6 WHERE id = ?1";

const DELETE_ENVIRONMENT: &str = "DELETE FROM network_environments WHERE id = ?1";

const SELECT_COOKIES: &str = "SELECT id, name, domain, host_only, path, secure, http_only, \
    expires_at, enabled, secret_parts, secret_slot, created_at, updated_at FROM network_cookies \
    ORDER BY domain ASC, path ASC, name ASC, id ASC";

const UPSERT_COOKIE: &str = "INSERT INTO network_cookies \
    (id, name, domain, host_only, path, secure, http_only, expires_at, enabled, secret_parts, \
     secret_slot, created_at, updated_at) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13) \
    ON CONFLICT (name, domain, path) DO UPDATE SET \
        host_only = excluded.host_only, \
        secure = excluded.secure, \
        http_only = excluded.http_only, \
        expires_at = excluded.expires_at, \
        enabled = excluded.enabled, \
        secret_parts = excluded.secret_parts, \
        secret_slot = excluded.secret_slot, \
        updated_at = excluded.updated_at";

const UPDATE_COOKIE: &str = "UPDATE network_cookies \
    SET name = ?2, domain = ?3, host_only = ?4, path = ?5, secure = ?6, http_only = ?7, \
        expires_at = ?8, enabled = ?9, secret_parts = ?10, secret_slot = ?11, updated_at = ?12 \
    WHERE id = ?1";

const DELETE_COOKIE: &str = "DELETE FROM network_cookies WHERE id = ?1";

const DELETE_ALL_COOKIES: &str = "DELETE FROM network_cookies";

const SELECT_GLOBAL_VARIABLES: &str = "SELECT document, secret_parts, secret_slot, updated_at \
    FROM network_global_variables WHERE id = 1";

const UPSERT_GLOBAL_VARIABLES: &str = "INSERT INTO network_global_variables \
    (id, document, secret_parts, secret_slot, updated_at) VALUES (1, ?1, ?2, ?3, ?4) \
    ON CONFLICT (id) DO UPDATE SET \
        document = excluded.document, \
        secret_parts = excluded.secret_parts, \
        secret_slot = excluded.secret_slot, \
        updated_at = excluded.updated_at";

const STATEMENTS: &[&str] = &[
    SELECT_COLLECTIONS,
    INSERT_COLLECTION,
    UPDATE_COLLECTION,
    SET_COLLECTION_ENABLED,
    SET_COLLECTION_ACTIVE_ENV,
    DELETE_COLLECTION,
    SELECT_FOLDERS,
    INSERT_FOLDER,
    RENAME_FOLDER,
    DELETE_FOLDER,
    SELECT_REQUESTS,
    INSERT_REQUEST,
    UPDATE_REQUEST,
    DELETE_REQUEST,
    SELECT_ENVIRONMENTS,
    INSERT_ENVIRONMENT,
    UPDATE_ENVIRONMENT,
    DELETE_ENVIRONMENT,
    SELECT_COOKIES,
    UPSERT_COOKIE,
    UPDATE_COOKIE,
    DELETE_COOKIE,
    DELETE_ALL_COOKIES,
    SELECT_GLOBAL_VARIABLES,
    UPSERT_GLOBAL_VARIABLES,
];

/// The store module declared by `NetworkPlugin`.
pub const MODULE: StoreModule = StoreModule::new(PREFIX, MIGRATIONS, STATEMENTS);

const CONSTRAINT_COLLECTION_NAME: &str = "collection_name_taken";
const CONSTRAINT_ENVIRONMENT_NAME: &str = "environment_name_taken";

/// Non-secret JSON document persisted in `network_collections.document`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CollectionDocument {
    #[serde(default)]
    pub(crate) headers: Vec<KeyValueEntry>,
    #[serde(default)]
    pub(crate) query_params: Vec<KeyValueEntry>,
    #[serde(default)]
    pub(crate) auth: AuthConfig,
    #[serde(default)]
    pub(crate) variables: Vec<VariableEntry>,
}

/// Non-secret JSON document persisted in `network_requests.document`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequestDocument {
    #[serde(default)]
    pub(crate) path_params: Vec<KeyValueEntry>,
    #[serde(default)]
    pub(crate) query_params: Vec<KeyValueEntry>,
    #[serde(default)]
    pub(crate) headers: Vec<KeyValueEntry>,
    #[serde(default)]
    pub(crate) auth: AuthConfig,
    #[serde(default)]
    pub(crate) body: BodyConfig,
}

/// Non-secret JSON document persisted in `network_environments.document` and
/// `network_global_variables.document`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VariablesDocument {
    #[serde(default)]
    pub(crate) variables: Vec<VariableEntry>,
}

/// Row loaded from `network_collections`.
#[derive(Debug, Clone)]
pub(crate) struct CollectionRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) protocol: Protocol,
    pub(crate) base_url: String,
    pub(crate) enabled: bool,
    pub(crate) active_environment_id: Option<String>,
    pub(crate) auto_cookies: bool,
    pub(crate) document: CollectionDocument,
    pub(crate) secret_parts: u32,
    pub(crate) secret_slot: String,
    pub(crate) created_at: UtcMillis,
    pub(crate) updated_at: UtcMillis,
}

/// Row loaded from `network_folders`.
#[derive(Debug, Clone)]
pub(crate) struct FolderRow {
    pub(crate) id: String,
    pub(crate) collection_id: String,
    pub(crate) parent_id: Option<String>,
    pub(crate) name: String,
    pub(crate) sort_order: u32,
    pub(crate) created_at: UtcMillis,
    pub(crate) updated_at: UtcMillis,
}

/// Row loaded from `network_requests`.
#[derive(Debug, Clone)]
pub(crate) struct RequestRow {
    pub(crate) id: String,
    pub(crate) collection_id: String,
    pub(crate) folder_id: Option<String>,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) protocol: Protocol,
    pub(crate) method: HttpMethod,
    pub(crate) url_template: String,
    pub(crate) timeout_seconds: u32,
    pub(crate) use_cookies: bool,
    pub(crate) sort_order: u32,
    pub(crate) document: RequestDocument,
    pub(crate) secret_parts: u32,
    pub(crate) secret_slot: String,
    pub(crate) created_at: UtcMillis,
    pub(crate) updated_at: UtcMillis,
}

/// Row loaded from `network_environments`.
#[derive(Debug, Clone)]
pub(crate) struct EnvironmentRow {
    pub(crate) id: String,
    pub(crate) collection_id: String,
    pub(crate) name: String,
    pub(crate) document: VariablesDocument,
    pub(crate) secret_parts: u32,
    pub(crate) secret_slot: String,
    pub(crate) created_at: UtcMillis,
    pub(crate) updated_at: UtcMillis,
}

/// Row loaded from `network_cookies`.
#[derive(Debug, Clone)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "RFC 6265 cookie attributes (host_only, secure, http_only) plus enabled toggle"
)]
pub(crate) struct CookieRow {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) domain: String,
    pub(crate) host_only: bool,
    pub(crate) path: String,
    pub(crate) secure: bool,
    pub(crate) http_only: bool,
    pub(crate) expires_at: Option<UtcMillis>,
    pub(crate) enabled: bool,
    pub(crate) secret_parts: u32,
    pub(crate) secret_slot: String,
    pub(crate) created_at: UtcMillis,
    pub(crate) updated_at: UtcMillis,
}

/// Row loaded from `network_global_variables`.
#[derive(Debug, Clone, Default)]
pub(crate) struct GlobalVariablesRow {
    pub(crate) document: VariablesDocument,
    pub(crate) secret_parts: u32,
    pub(crate) secret_slot: String,
}

/// All raw SQLite rows loaded on plugin startup.
#[derive(Debug, Clone, Default)]
pub(crate) struct StoredTables {
    pub(crate) collections: Vec<CollectionRow>,
    pub(crate) folders: Vec<FolderRow>,
    pub(crate) requests: Vec<RequestRow>,
    pub(crate) environments: Vec<EnvironmentRow>,
    pub(crate) cookies: Vec<CookieRow>,
    pub(crate) global_variables: GlobalVariablesRow,
}

/// Typed wrapper over the plugin's `store` service.
#[derive(Debug, Clone)]
pub(crate) struct NetworkStore {
    api: Arc<dyn ModuleStoreApi>,
}

impl NetworkStore {
    pub(crate) fn new(api: Arc<dyn ModuleStoreApi>) -> Self {
        Self { api }
    }

    /// Loads every non-secret table on plugin start.
    pub(crate) async fn load_all(&self) -> Result<StoredTables, StoreError> {
        let collections = self
            .api
            .query(Statement::new(SELECT_COLLECTIONS))
            .await?
            .iter()
            .map(decode_collection)
            .collect::<Result<Vec<_>, _>>()?;
        let folders = self
            .api
            .query(Statement::new(SELECT_FOLDERS))
            .await?
            .iter()
            .map(decode_folder)
            .collect::<Result<Vec<_>, _>>()?;
        let requests = self
            .api
            .query(Statement::new(SELECT_REQUESTS))
            .await?
            .iter()
            .map(decode_request)
            .collect::<Result<Vec<_>, _>>()?;
        let environments = self
            .api
            .query(Statement::new(SELECT_ENVIRONMENTS))
            .await?
            .iter()
            .map(decode_environment)
            .collect::<Result<Vec<_>, _>>()?;
        let cookies = self
            .api
            .query(Statement::new(SELECT_COOKIES))
            .await?
            .iter()
            .map(decode_cookie)
            .collect::<Result<Vec<_>, _>>()?;
        let global_rows = self
            .api
            .query(Statement::new(SELECT_GLOBAL_VARIABLES))
            .await?;
        let global_variables = match global_rows.first() {
            Some(row) => decode_global_variables(row)?,
            None => GlobalVariablesRow::default(),
        };

        Ok(StoredTables {
            collections,
            folders,
            requests,
            environments,
            cookies,
            global_variables,
        })
    }

    pub(crate) async fn insert_collection(&self, row: &CollectionRow) -> Result<(), NetworkError> {
        let doc =
            serde_json::to_string(&row.document).map_err(|_err| NetworkError::StorageFailed)?;
        let stmt = Statement::new(INSERT_COLLECTION)
            .bind(row.id.clone())
            .bind(row.name.clone())
            .bind(row.description.clone())
            .bind(row.protocol.as_str())
            .bind(row.base_url.clone())
            .bind(row.enabled)
            .bind(row.active_environment_id.clone())
            .bind(row.auto_cookies)
            .bind(doc)
            .bind(row.secret_parts)
            .bind(row.secret_slot.clone())
            .bind(row.created_at.as_millis())
            .bind(row.updated_at.as_millis());
        let batch = WriteBatch::new()
            .push(stmt, Expect::Changes(1))
            .on_constraint(
                Constraint::Key {
                    table: "network_collections",
                    columns: &["name"],
                },
                CONSTRAINT_COLLECTION_NAME,
            );
        self.api
            .write(batch, Durability::Full)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    pub(crate) async fn update_collection(&self, row: &CollectionRow) -> Result<(), NetworkError> {
        let doc =
            serde_json::to_string(&row.document).map_err(|_err| NetworkError::StorageFailed)?;
        let stmt = Statement::new(UPDATE_COLLECTION)
            .bind(row.id.clone())
            .bind(row.name.clone())
            .bind(row.description.clone())
            .bind(row.base_url.clone())
            .bind(row.enabled)
            .bind(row.active_environment_id.clone())
            .bind(row.auto_cookies)
            .bind(doc)
            .bind(row.secret_parts)
            .bind(row.secret_slot.clone())
            .bind(row.updated_at.as_millis());
        let batch = WriteBatch::new()
            .push(stmt, Expect::Changes(1))
            .on_constraint(
                Constraint::Key {
                    table: "network_collections",
                    columns: &["name"],
                },
                CONSTRAINT_COLLECTION_NAME,
            );
        self.api
            .write(batch, Durability::Full)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    pub(crate) async fn set_collection_enabled(
        &self,
        id: &str,
        enabled: bool,
        updated_at: UtcMillis,
    ) -> Result<(), NetworkError> {
        let stmt = Statement::new(SET_COLLECTION_ENABLED)
            .bind(id)
            .bind(enabled)
            .bind(updated_at.as_millis());
        let batch = WriteBatch::new().push(stmt, Expect::Changes(1));
        self.api
            .write(batch, Durability::Full)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    pub(crate) async fn set_active_environment(
        &self,
        collection_id: &str,
        environment_id: Option<&str>,
        updated_at: UtcMillis,
    ) -> Result<(), NetworkError> {
        let stmt = Statement::new(SET_COLLECTION_ACTIVE_ENV)
            .bind(collection_id)
            .bind(environment_id.map(str::to_owned))
            .bind(updated_at.as_millis());
        let batch = WriteBatch::new().push(stmt, Expect::Changes(1));
        self.api
            .write(batch, Durability::Full)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    pub(crate) async fn delete_collection(&self, id: &str) -> Result<(), NetworkError> {
        let stmt = Statement::new(DELETE_COLLECTION).bind(id);
        let batch = WriteBatch::new().push(stmt, Expect::Any);
        self.api
            .write(batch, Durability::Erase)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    pub(crate) async fn insert_folder(&self, row: &FolderRow) -> Result<(), NetworkError> {
        let stmt = Statement::new(INSERT_FOLDER)
            .bind(row.id.clone())
            .bind(row.collection_id.clone())
            .bind(row.parent_id.clone())
            .bind(row.name.clone())
            .bind(row.sort_order)
            .bind(row.created_at.as_millis())
            .bind(row.updated_at.as_millis());
        let batch = WriteBatch::new().push(stmt, Expect::Changes(1));
        self.api
            .write(batch, Durability::Full)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    pub(crate) async fn rename_folder(
        &self,
        id: &str,
        name: &str,
        updated_at: UtcMillis,
    ) -> Result<(), NetworkError> {
        let stmt = Statement::new(RENAME_FOLDER)
            .bind(id)
            .bind(name)
            .bind(updated_at.as_millis());
        let batch = WriteBatch::new().push(stmt, Expect::Changes(1));
        self.api
            .write(batch, Durability::Full)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    pub(crate) async fn delete_folder(&self, id: &str) -> Result<(), NetworkError> {
        let stmt = Statement::new(DELETE_FOLDER).bind(id);
        let batch = WriteBatch::new().push(stmt, Expect::Any);
        self.api
            .write(batch, Durability::Erase)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    pub(crate) async fn insert_request(&self, row: &RequestRow) -> Result<(), NetworkError> {
        let doc =
            serde_json::to_string(&row.document).map_err(|_err| NetworkError::StorageFailed)?;
        let stmt = Statement::new(INSERT_REQUEST)
            .bind(row.id.clone())
            .bind(row.collection_id.clone())
            .bind(row.folder_id.clone())
            .bind(row.name.clone())
            .bind(row.description.clone())
            .bind(row.protocol.as_str())
            .bind(row.method.as_str())
            .bind(row.url_template.clone())
            .bind(row.timeout_seconds)
            .bind(row.use_cookies)
            .bind(row.sort_order)
            .bind(doc)
            .bind(row.secret_parts)
            .bind(row.secret_slot.clone())
            .bind(row.created_at.as_millis())
            .bind(row.updated_at.as_millis());
        let batch = WriteBatch::new().push(stmt, Expect::Changes(1));
        self.api
            .write(batch, Durability::Full)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    pub(crate) async fn update_request(&self, row: &RequestRow) -> Result<(), NetworkError> {
        let doc =
            serde_json::to_string(&row.document).map_err(|_err| NetworkError::StorageFailed)?;
        let stmt = Statement::new(UPDATE_REQUEST)
            .bind(row.id.clone())
            .bind(row.folder_id.clone())
            .bind(row.name.clone())
            .bind(row.description.clone())
            .bind(row.method.as_str())
            .bind(row.url_template.clone())
            .bind(row.timeout_seconds)
            .bind(row.use_cookies)
            .bind(doc)
            .bind(row.secret_parts)
            .bind(row.secret_slot.clone())
            .bind(row.updated_at.as_millis());
        let batch = WriteBatch::new().push(stmt, Expect::Changes(1));
        self.api
            .write(batch, Durability::Full)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    pub(crate) async fn delete_request(&self, id: &str) -> Result<(), NetworkError> {
        let stmt = Statement::new(DELETE_REQUEST).bind(id);
        let batch = WriteBatch::new().push(stmt, Expect::Any);
        self.api
            .write(batch, Durability::Erase)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    pub(crate) async fn insert_environment(
        &self,
        row: &EnvironmentRow,
    ) -> Result<(), NetworkError> {
        let doc =
            serde_json::to_string(&row.document).map_err(|_err| NetworkError::StorageFailed)?;
        let stmt = Statement::new(INSERT_ENVIRONMENT)
            .bind(row.id.clone())
            .bind(row.collection_id.clone())
            .bind(row.name.clone())
            .bind(doc)
            .bind(row.secret_parts)
            .bind(row.secret_slot.clone())
            .bind(row.created_at.as_millis())
            .bind(row.updated_at.as_millis());
        let batch = WriteBatch::new()
            .push(stmt, Expect::Changes(1))
            .on_constraint(
                Constraint::Key {
                    table: "network_environments",
                    columns: &["collection_id", "name"],
                },
                CONSTRAINT_ENVIRONMENT_NAME,
            );
        self.api
            .write(batch, Durability::Full)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    pub(crate) async fn update_environment(
        &self,
        row: &EnvironmentRow,
    ) -> Result<(), NetworkError> {
        let doc =
            serde_json::to_string(&row.document).map_err(|_err| NetworkError::StorageFailed)?;
        let stmt = Statement::new(UPDATE_ENVIRONMENT)
            .bind(row.id.clone())
            .bind(row.name.clone())
            .bind(doc)
            .bind(row.secret_parts)
            .bind(row.secret_slot.clone())
            .bind(row.updated_at.as_millis());
        let batch = WriteBatch::new()
            .push(stmt, Expect::Changes(1))
            .on_constraint(
                Constraint::Key {
                    table: "network_environments",
                    columns: &["collection_id", "name"],
                },
                CONSTRAINT_ENVIRONMENT_NAME,
            );
        self.api
            .write(batch, Durability::Full)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    pub(crate) async fn delete_environment(&self, id: &str) -> Result<(), NetworkError> {
        let stmt = Statement::new(DELETE_ENVIRONMENT).bind(id);
        let batch = WriteBatch::new().push(stmt, Expect::Any);
        self.api
            .write(batch, Durability::Erase)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    pub(crate) async fn upsert_cookie(&self, row: &CookieRow) -> Result<(), NetworkError> {
        let stmt = Statement::new(UPSERT_COOKIE)
            .bind(row.id.clone())
            .bind(row.name.clone())
            .bind(row.domain.clone())
            .bind(row.host_only)
            .bind(row.path.clone())
            .bind(row.secure)
            .bind(row.http_only)
            .bind(row.expires_at.map(UtcMillis::as_millis))
            .bind(row.enabled)
            .bind(row.secret_parts)
            .bind(row.secret_slot.clone())
            .bind(row.created_at.as_millis())
            .bind(row.updated_at.as_millis());
        let batch = WriteBatch::new().push(stmt, Expect::Any);
        self.api
            .write(batch, Durability::Full)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    pub(crate) async fn update_cookie(&self, row: &CookieRow) -> Result<(), NetworkError> {
        let stmt = Statement::new(UPDATE_COOKIE)
            .bind(row.id.clone())
            .bind(row.name.clone())
            .bind(row.domain.clone())
            .bind(row.host_only)
            .bind(row.path.clone())
            .bind(row.secure)
            .bind(row.http_only)
            .bind(row.expires_at.map(UtcMillis::as_millis))
            .bind(row.enabled)
            .bind(row.secret_parts)
            .bind(row.secret_slot.clone())
            .bind(row.updated_at.as_millis());
        let batch = WriteBatch::new().push(stmt, Expect::Changes(1));
        self.api
            .write(batch, Durability::Full)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    pub(crate) async fn delete_cookie(&self, id: &str) -> Result<(), NetworkError> {
        let stmt = Statement::new(DELETE_COOKIE).bind(id);
        let batch = WriteBatch::new().push(stmt, Expect::Any);
        self.api
            .write(batch, Durability::Erase)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    pub(crate) async fn delete_all_cookies(&self) -> Result<(), NetworkError> {
        let stmt = Statement::new(DELETE_ALL_COOKIES);
        let batch = WriteBatch::new().push(stmt, Expect::Any);
        self.api
            .write(batch, Durability::Erase)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    pub(crate) async fn save_global_variables(
        &self,
        document: &VariablesDocument,
        secret_parts: u32,
        secret_slot: &str,
        updated_at: UtcMillis,
    ) -> Result<(), NetworkError> {
        let doc = serde_json::to_string(document).map_err(|_err| NetworkError::StorageFailed)?;
        let stmt = Statement::new(UPSERT_GLOBAL_VARIABLES)
            .bind(doc)
            .bind(secret_parts)
            .bind(secret_slot.to_owned())
            .bind(updated_at.as_millis());
        let batch = WriteBatch::new().push(stmt, Expect::Changes(1));
        self.api
            .write(batch, Durability::Full)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }

    /// Inserts a full imported collection, its folders, environments and
    /// requests inside a single atomic transaction.
    pub(crate) async fn insert_imported_collection(
        &self,
        collection: &CollectionRow,
        folders: &[FolderRow],
        environments: &[EnvironmentRow],
        requests: &[RequestRow],
    ) -> Result<(), NetworkError> {
        let col_doc = serde_json::to_string(&collection.document)
            .map_err(|_err| NetworkError::StorageFailed)?;
        let mut batch = WriteBatch::new()
            .push(
                Statement::new(INSERT_COLLECTION)
                    .bind(collection.id.clone())
                    .bind(collection.name.clone())
                    .bind(collection.description.clone())
                    .bind(collection.protocol.as_str())
                    .bind(collection.base_url.clone())
                    .bind(collection.enabled)
                    .bind(collection.active_environment_id.clone())
                    .bind(collection.auto_cookies)
                    .bind(col_doc)
                    .bind(collection.secret_parts)
                    .bind(collection.secret_slot.clone())
                    .bind(collection.created_at.as_millis())
                    .bind(collection.updated_at.as_millis()),
                Expect::Changes(1),
            )
            .on_constraint(
                Constraint::Key {
                    table: "network_collections",
                    columns: &["name"],
                },
                CONSTRAINT_COLLECTION_NAME,
            );

        for folder in folders {
            batch = batch.push(
                Statement::new(INSERT_FOLDER)
                    .bind(folder.id.clone())
                    .bind(folder.collection_id.clone())
                    .bind(folder.parent_id.clone())
                    .bind(folder.name.clone())
                    .bind(folder.sort_order)
                    .bind(folder.created_at.as_millis())
                    .bind(folder.updated_at.as_millis()),
                Expect::Changes(1),
            );
        }

        for env in environments {
            let env_doc =
                serde_json::to_string(&env.document).map_err(|_err| NetworkError::StorageFailed)?;
            batch = batch.push(
                Statement::new(INSERT_ENVIRONMENT)
                    .bind(env.id.clone())
                    .bind(env.collection_id.clone())
                    .bind(env.name.clone())
                    .bind(env_doc)
                    .bind(env.secret_parts)
                    .bind(env.secret_slot.clone())
                    .bind(env.created_at.as_millis())
                    .bind(env.updated_at.as_millis()),
                Expect::Changes(1),
            );
        }

        for req in requests {
            let req_doc =
                serde_json::to_string(&req.document).map_err(|_err| NetworkError::StorageFailed)?;
            batch = batch.push(
                Statement::new(INSERT_REQUEST)
                    .bind(req.id.clone())
                    .bind(req.collection_id.clone())
                    .bind(req.folder_id.clone())
                    .bind(req.name.clone())
                    .bind(req.description.clone())
                    .bind(req.protocol.as_str())
                    .bind(req.method.as_str())
                    .bind(req.url_template.clone())
                    .bind(req.timeout_seconds)
                    .bind(req.use_cookies)
                    .bind(req.sort_order)
                    .bind(req_doc)
                    .bind(req.secret_parts)
                    .bind(req.secret_slot.clone())
                    .bind(req.created_at.as_millis())
                    .bind(req.updated_at.as_millis()),
                Expect::Changes(1),
            );
        }

        self.api
            .write(batch, Durability::Full)
            .await
            .map_err(map_store_error)?;
        Ok(())
    }
}

fn map_store_error(error: StoreError) -> NetworkError {
    match error {
        StoreError::Constraint { code } if code == CONSTRAINT_COLLECTION_NAME => {
            NetworkError::NameTaken
        }
        StoreError::Constraint { code } if code == CONSTRAINT_ENVIRONMENT_NAME => {
            NetworkError::EnvironmentNameTaken
        }
        other => {
            tracing::error!(error.code = other.code(), "network store write failed");
            NetworkError::StorageFailed
        }
    }
}

fn get_u32(row: &Row, column: usize) -> Result<u32, StoreError> {
    let raw: i64 = row.get(column)?;
    u32::try_from(raw).map_err(|_err| StoreError::InvalidValue { column })
}

fn decode_collection(row: &Row) -> Result<CollectionRow, StoreError> {
    let protocol_text: String = row.get(3)?;
    let protocol = Protocol::parse(&protocol_text).ok_or(StoreError::InvalidValue { column: 3 })?;
    let doc_text: String = row.get(8)?;
    let document: CollectionDocument =
        serde_json::from_str(&doc_text).map_err(|_err| StoreError::InvalidValue { column: 8 })?;
    Ok(CollectionRow {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        protocol,
        base_url: row.get(4)?,
        enabled: row.get(5)?,
        active_environment_id: row.get(6)?,
        auto_cookies: row.get(7)?,
        document,
        secret_parts: get_u32(row, 9)?,
        secret_slot: row.get(10)?,
        created_at: UtcMillis::from_millis(row.get(11)?),
        updated_at: UtcMillis::from_millis(row.get(12)?),
    })
}

fn decode_folder(row: &Row) -> Result<FolderRow, StoreError> {
    Ok(FolderRow {
        id: row.get(0)?,
        collection_id: row.get(1)?,
        parent_id: row.get(2)?,
        name: row.get(3)?,
        sort_order: get_u32(row, 4)?,
        created_at: UtcMillis::from_millis(row.get(5)?),
        updated_at: UtcMillis::from_millis(row.get(6)?),
    })
}

fn decode_request(row: &Row) -> Result<RequestRow, StoreError> {
    let protocol_text: String = row.get(5)?;
    let protocol = Protocol::parse(&protocol_text).ok_or(StoreError::InvalidValue { column: 5 })?;
    let method_text: String = row.get(6)?;
    let method = HttpMethod::parse(&method_text).ok_or(StoreError::InvalidValue { column: 6 })?;
    let doc_text: String = row.get(11)?;
    let document: RequestDocument =
        serde_json::from_str(&doc_text).map_err(|_err| StoreError::InvalidValue { column: 11 })?;
    Ok(RequestRow {
        id: row.get(0)?,
        collection_id: row.get(1)?,
        folder_id: row.get(2)?,
        name: row.get(3)?,
        description: row.get(4)?,
        protocol,
        method,
        url_template: row.get(7)?,
        timeout_seconds: get_u32(row, 8)?,
        use_cookies: row.get(9)?,
        sort_order: get_u32(row, 10)?,
        document,
        secret_parts: get_u32(row, 12)?,
        secret_slot: row.get(13)?,
        created_at: UtcMillis::from_millis(row.get(14)?),
        updated_at: UtcMillis::from_millis(row.get(15)?),
    })
}

fn decode_environment(row: &Row) -> Result<EnvironmentRow, StoreError> {
    let doc_text: String = row.get(3)?;
    let document: VariablesDocument =
        serde_json::from_str(&doc_text).map_err(|_err| StoreError::InvalidValue { column: 3 })?;
    Ok(EnvironmentRow {
        id: row.get(0)?,
        collection_id: row.get(1)?,
        name: row.get(2)?,
        document,
        secret_parts: get_u32(row, 4)?,
        secret_slot: row.get(5)?,
        created_at: UtcMillis::from_millis(row.get(6)?),
        updated_at: UtcMillis::from_millis(row.get(7)?),
    })
}

fn decode_cookie(row: &Row) -> Result<CookieRow, StoreError> {
    let expires: Option<i64> = row.get(7)?;
    Ok(CookieRow {
        id: row.get(0)?,
        name: row.get(1)?,
        domain: row.get(2)?,
        host_only: row.get(3)?,
        path: row.get(4)?,
        secure: row.get(5)?,
        http_only: row.get(6)?,
        expires_at: expires.map(UtcMillis::from_millis),
        enabled: row.get(8)?,
        secret_parts: get_u32(row, 9)?,
        secret_slot: row.get(10)?,
        created_at: UtcMillis::from_millis(row.get(11)?),
        updated_at: UtcMillis::from_millis(row.get(12)?),
    })
}

fn decode_global_variables(row: &Row) -> Result<GlobalVariablesRow, StoreError> {
    let doc_text: String = row.get(0)?;
    let document: VariablesDocument =
        serde_json::from_str(&doc_text).map_err(|_err| StoreError::InvalidValue { column: 0 })?;
    Ok(GlobalVariablesRow {
        document,
        secret_parts: get_u32(row, 1)?,
        secret_slot: row.get(2)?,
    })
}
