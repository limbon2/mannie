//! The plugin's table, `ssh_hosts` (plugins/ssh/docs/README.md, data): one
//! row for each host, with its pinned key and the key that it showed last.
//! The passwords are not here: the credential store keeps them, and the row
//! only says whether one is stored.
//!
//! The pinned key is complete or absent, which a CHECK keeps, and so is the
//! key seen last. Trusting a key pins only the one that the last scan saw,
//! by its fingerprint, so a key that changed between the scan and the
//! owner's click is never pinned.

use std::sync::Arc;

use mannie_plugin_sdk::contracts::UtcMillis;
use mannie_plugin_sdk::services::store::{
    Constraint, Durability, Expect, Migration, ModuleStoreApi, Row, Statement, StoreError,
    StoreModule, WriteBatch,
};

use crate::commands::{HostKeyView, HostStatus, HostView};
use crate::rules::endpoint;

/// The prefix of the plugin's tables, which names its module of the store.
pub(crate) const PREFIX: &str = "ssh";

const MIGRATIONS: &[Migration] = &[Migration::new(
    1,
    "create the table of the hosts",
    "CREATE TABLE ssh_hosts (
        id TEXT PRIMARY KEY NOT NULL,
        name TEXT NOT NULL UNIQUE COLLATE NOCASE,
        note TEXT NOT NULL,
        address TEXT NOT NULL,
        port INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
        username TEXT NOT NULL,
        password_set INTEGER NOT NULL CHECK (password_set IN (0, 1)),
        key_algorithm TEXT,
        key_openssh TEXT,
        key_fingerprint TEXT,
        key_at INTEGER,
        seen_algorithm TEXT,
        seen_openssh TEXT,
        seen_fingerprint TEXT,
        seen_at INTEGER,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        CONSTRAINT ssh_hosts_key_complete CHECK (
            (key_algorithm IS NULL) = (key_openssh IS NULL)
            AND (key_openssh IS NULL) = (key_fingerprint IS NULL)
            AND (key_fingerprint IS NULL) = (key_at IS NULL)
        ),
        CONSTRAINT ssh_hosts_seen_complete CHECK (
            (seen_algorithm IS NULL) = (seen_openssh IS NULL)
            AND (seen_openssh IS NULL) = (seen_fingerprint IS NULL)
            AND (seen_fingerprint IS NULL) = (seen_at IS NULL)
        )
    ) STRICT",
)];

const SELECT_HOSTS: &str = "SELECT id, name, note, address, port, username, password_set, \
    key_algorithm, key_openssh, key_fingerprint, key_at, \
    seen_algorithm, seen_openssh, seen_fingerprint, seen_at \
    FROM ssh_hosts ORDER BY name COLLATE NOCASE, id";

const INSERT_HOST: &str = "INSERT INTO ssh_hosts (id, name, note, address, port, username, \
    password_set, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, ?7)";

const SET_PASSWORD_SET: &str =
    "UPDATE ssh_hosts SET password_set = ?2, updated_at = ?3 WHERE id = ?1";

const UPDATE_HOST: &str = "UPDATE ssh_hosts SET name = ?2, note = ?3, address = ?4, port = ?5, \
    username = ?6, updated_at = ?7 WHERE id = ?1";

const FORGET_KEYS: &str = "UPDATE ssh_hosts SET key_algorithm = NULL, key_openssh = NULL, \
    key_fingerprint = NULL, key_at = NULL, seen_algorithm = NULL, seen_openssh = NULL, \
    seen_fingerprint = NULL, seen_at = NULL WHERE id = ?1";

const RECORD_SEEN_KEY: &str = "UPDATE ssh_hosts SET seen_algorithm = ?2, seen_openssh = ?3, \
    seen_fingerprint = ?4, seen_at = ?5 WHERE id = ?1";

const TRUST_SEEN_KEY: &str = "UPDATE ssh_hosts SET key_algorithm = seen_algorithm, \
    key_openssh = seen_openssh, key_fingerprint = seen_fingerprint, key_at = ?3, \
    updated_at = ?3 WHERE id = ?1 AND seen_fingerprint = ?2";

const DELETE_HOST: &str = "DELETE FROM ssh_hosts WHERE id = ?1";

/// The plugin's module of the store.
pub const MODULE: StoreModule = StoreModule::new(
    PREFIX,
    MIGRATIONS,
    &[
        SELECT_HOSTS,
        INSERT_HOST,
        SET_PASSWORD_SET,
        UPDATE_HOST,
        FORGET_KEYS,
        RECORD_SEEN_KEY,
        TRUST_SEEN_KEY,
        DELETE_HOST,
    ],
);

/// The code of a name that another host has, which a batch maps from the
/// table's unique name.
pub(crate) const NAME_TAKEN: &str = "name_taken";

/// A host key as the row keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredKey {
    /// The algorithm, such as `ssh-ed25519`.
    pub(crate) algorithm: String,
    /// The public key in OpenSSH's format, which the client compares.
    pub(crate) openssh: String,
    /// The SHA-256 fingerprint, which the owner compares.
    pub(crate) fingerprint: String,
    /// When the key was pinned or seen.
    pub(crate) at: i64,
}

impl StoredKey {
    fn view(&self) -> HostKeyView {
        HostKeyView {
            algorithm: self.algorithm.clone(),
            fingerprint: self.fingerprint.clone(),
            at: UtcMillis::from_millis(self.at),
        }
    }
}

/// A host as its row keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) note: String,
    pub(crate) address: String,
    pub(crate) port: u16,
    pub(crate) username: String,
    pub(crate) password_set: bool,
    /// The pinned key.
    pub(crate) key: Option<StoredKey>,
    /// The key that the host showed last.
    pub(crate) seen: Option<StoredKey>,
}

impl HostRecord {
    /// Where the host's commands run: `user@address:port`.
    pub(crate) fn endpoint(&self) -> String {
        endpoint(&self.username, &self.address, self.port)
    }

    /// The key that the host showed last, when it is not the pinned one.
    pub(crate) fn changed_key(&self) -> Option<&StoredKey> {
        let seen = self.seen.as_ref()?;
        let is_pinned = self
            .key
            .as_ref()
            .is_some_and(|key| key.fingerprint == seen.fingerprint);
        (!is_pinned).then_some(seen)
    }

    /// Whether the host can run commands.
    pub(crate) fn status(&self) -> HostStatus {
        if self.key.is_none() {
            return HostStatus::KeyNotChecked;
        }
        if self.changed_key().is_some() {
            return HostStatus::KeyChanged;
        }
        if !self.password_set {
            return HostStatus::PasswordMissing;
        }
        HostStatus::Ready
    }

    /// The host as the settings show it.
    pub(crate) fn view(&self) -> HostView {
        HostView {
            id: self.id.clone(),
            name: self.name.clone(),
            note: self.note.clone(),
            address: self.address.clone(),
            port: self.port,
            username: self.username.clone(),
            endpoint: self.endpoint(),
            password_set: self.password_set,
            key: self.key.as_ref().map(StoredKey::view),
            seen_key: self.changed_key().map(StoredKey::view),
            status: self.status(),
        }
    }
}

/// The fields of a host that the owner sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HostFields {
    pub(crate) name: String,
    pub(crate) note: String,
    pub(crate) address: String,
    pub(crate) port: u16,
    pub(crate) username: String,
}

/// The plugin's table, through its module of the store.
#[derive(Debug, Clone)]
pub(crate) struct HostsStore {
    store: Arc<dyn ModuleStoreApi>,
}

impl HostsStore {
    pub(crate) fn new(store: Arc<dyn ModuleStoreApi>) -> Self {
        Self { store }
    }

    /// Every host, by name.
    pub(crate) async fn hosts(&self) -> Result<Vec<HostRecord>, StoreError> {
        let rows = self.store.query(Statement::new(SELECT_HOSTS)).await?;
        rows.iter().map(record).collect()
    }

    /// Adds a host without a password. `NAME_TAKEN` names a taken name.
    pub(crate) async fn insert(
        &self,
        id: &str,
        fields: &HostFields,
        now: UtcMillis,
    ) -> Result<(), StoreError> {
        let insert =
            bind_fields(Statement::new(INSERT_HOST).bind(id), fields).bind(now.as_millis());
        let batch = WriteBatch::new()
            .push(insert, Expect::Changes(1))
            .on_constraint(name_key(), NAME_TAKEN);
        self.write(batch).await
    }

    /// Records whether the credential store holds the host's password.
    pub(crate) async fn set_password_set(
        &self,
        id: &str,
        password_set: bool,
        now: UtcMillis,
    ) -> Result<(), StoreError> {
        let update = Statement::new(SET_PASSWORD_SET)
            .bind(id)
            .bind(password_set)
            .bind(now.as_millis());
        self.write(WriteBatch::new().push(update, Expect::Changes(1)))
            .await
    }

    /// Changes a host's fields, and lets go of its keys when `forget_keys`
    /// says that its address or port changed, in one transaction.
    pub(crate) async fn update(
        &self,
        id: &str,
        fields: &HostFields,
        forget_keys: bool,
        now: UtcMillis,
    ) -> Result<(), StoreError> {
        let update =
            bind_fields(Statement::new(UPDATE_HOST).bind(id), fields).bind(now.as_millis());
        let mut batch = WriteBatch::new()
            .push(update, Expect::Changes(1))
            .on_constraint(name_key(), NAME_TAKEN);
        if forget_keys {
            batch = batch.push(Statement::new(FORGET_KEYS).bind(id), Expect::Changes(1));
        }
        self.write(batch).await
    }

    /// Records the key that the host showed.
    pub(crate) async fn record_seen(&self, id: &str, key: &StoredKey) -> Result<(), StoreError> {
        let update = Statement::new(RECORD_SEEN_KEY)
            .bind(id)
            .bind(key.algorithm.as_str())
            .bind(key.openssh.as_str())
            .bind(key.fingerprint.as_str())
            .bind(key.at);
        self.write(WriteBatch::new().push(update, Expect::Changes(1)))
            .await
    }

    /// Pins the key that the host showed last, when its fingerprint is
    /// `fingerprint`. Returns whether it was.
    pub(crate) async fn trust_seen(
        &self,
        id: &str,
        fingerprint: &str,
        now: UtcMillis,
    ) -> Result<bool, StoreError> {
        let update = Statement::new(TRUST_SEEN_KEY)
            .bind(id)
            .bind(fingerprint)
            .bind(now.as_millis());
        let changed = self
            .store
            .write(
                WriteBatch::new().push(update, Expect::Any),
                Durability::Full,
            )
            .await?;
        Ok(changed.first().is_some_and(|rows| *rows == 1))
    }

    /// Forgets a host. A host that is gone already is not a failure.
    pub(crate) async fn delete(&self, id: &str) -> Result<(), StoreError> {
        let delete = Statement::new(DELETE_HOST).bind(id);
        self.write(WriteBatch::new().push(delete, Expect::Any))
            .await
    }

    async fn write(&self, batch: WriteBatch) -> Result<(), StoreError> {
        self.store.write(batch, Durability::Full).await.map(drop)
    }
}

/// The unique name of the table, as SQLite reports its failure.
fn name_key() -> Constraint {
    Constraint::Key {
        table: "ssh_hosts",
        columns: &["name"],
    }
}

/// Binds the owner's fields, from `?2` to `?6`.
fn bind_fields(statement: Statement, fields: &HostFields) -> Statement {
    statement
        .bind(fields.name.as_str())
        .bind(fields.note.as_str())
        .bind(fields.address.as_str())
        .bind(u32::from(fields.port))
        .bind(fields.username.as_str())
}

/// A host from a row of `SELECT_HOSTS`.
fn record(row: &Row) -> Result<HostRecord, StoreError> {
    let port: i64 = row.get(4)?;
    Ok(HostRecord {
        id: row.get(0)?,
        name: row.get(1)?,
        note: row.get(2)?,
        address: row.get(3)?,
        port: u16::try_from(port)
            .map_err(|_out_of_range| StoreError::InvalidValue { column: 4 })?,
        username: row.get(5)?,
        password_set: row.get(6)?,
        key: stored_key(row, 7)?,
        seen: stored_key(row, 11)?,
    })
}

/// A key from four columns, starting at `first`: the algorithm, the key, the
/// fingerprint and the time.
fn stored_key(row: &Row, first: usize) -> Result<Option<StoredKey>, StoreError> {
    let algorithm: Option<String> = row.get(first)?;
    let openssh: Option<String> = row.get(first + 1)?;
    let fingerprint: Option<String> = row.get(first + 2)?;
    let at: Option<i64> = row.get(first + 3)?;
    Ok(match (algorithm, openssh, fingerprint, at) {
        (Some(algorithm), Some(openssh), Some(fingerprint), Some(at)) => Some(StoredKey {
            algorithm,
            openssh,
            fingerprint,
            at,
        }),
        _incomplete => None,
    })
}
