//! The registrations of the owner's SQLite files on this computer
//! (ADR-0056, decision 2): each plugin's own, by key, with the owner's name
//! for the file, its canonical path, its identity and whether the owner
//! marked it read-only. The plugin and its view handle keys and never paths.

use std::path::PathBuf;

use mannie_types::clock::UtcMillis;
use mannie_types::ids::DatabaseKey;
use rusqlite::{OptionalExtension, params};

use crate::error::StoreError;
use crate::writer::TransactError;
use crate::{Durability, Store};

/// The most databases that one plugin registers.
pub const MAX_REGISTRATIONS: usize = 256;

const SELECT_ALL: &str = "SELECT key, name, path, identity, read_only, created_at, updated_at \
     FROM dbfiles_registrations WHERE plugin = ?1 ORDER BY name, key";
const SELECT_ONE: &str = "SELECT key, name, path, identity, read_only, created_at, updated_at \
     FROM dbfiles_registrations WHERE plugin = ?1 AND key = ?2";
const COUNT: &str = "SELECT count(*) FROM dbfiles_registrations WHERE plugin = ?1";
const SELECT_PATH: &str = "SELECT key FROM dbfiles_registrations WHERE plugin = ?1 AND path = ?2";
const INSERT: &str = "INSERT INTO dbfiles_registrations \
     (plugin, key, name, path, identity, read_only, created_at, updated_at) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)";
const RENAME: &str = "UPDATE dbfiles_registrations SET name = ?3, updated_at = ?4 \
     WHERE plugin = ?1 AND key = ?2";
const MARK: &str = "UPDATE dbfiles_registrations SET read_only = ?3, updated_at = ?4 \
     WHERE plugin = ?1 AND key = ?2";
const IDENTIFY: &str = "UPDATE dbfiles_registrations SET identity = ?3, updated_at = ?4 \
     WHERE plugin = ?1 AND key = ?2";
const DELETE: &str = "DELETE FROM dbfiles_registrations WHERE plugin = ?1 AND key = ?2";

#[cfg(test)]
pub(crate) const STATEMENTS: &[&str] = &[
    SELECT_ALL,
    SELECT_ONE,
    COUNT,
    SELECT_PATH,
    INSERT,
    RENAME,
    MARK,
    IDENTIFY,
    DELETE,
];

/// A file that a plugin registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registration {
    /// The plugin's key for it.
    pub key: DatabaseKey,
    /// The owner's name for it.
    pub name: String,
    /// Its canonical path.
    pub path: PathBuf,
    /// Its identity when the owner registered it, or last accepted it.
    pub identity: String,
    /// Whether the owner marked it read-only, which refuses writes and
    /// schema changes.
    pub read_only: bool,
    /// When it was registered.
    pub created_at: UtcMillis,
    /// When its registration last changed.
    pub updated_at: UtcMillis,
}

/// What a new registration holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRegistration {
    /// The key that the runtime made for it.
    pub key: DatabaseKey,
    /// The owner's name for it.
    pub name: String,
    /// Its canonical path, from the file's checks.
    pub path: PathBuf,
    /// Its identity, from the file's checks.
    pub identity: String,
    /// Whether the owner marked it read-only.
    pub read_only: bool,
}

/// Why a registration could not change.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// The plugin already registered the file, under this key.
    #[error("the file is already registered")]
    AlreadyRegistered {
        /// The key of its registration.
        key: DatabaseKey,
    },
    /// The plugin registered as many files as it may.
    #[error("the plugin registered {MAX_REGISTRATIONS} files already")]
    TooMany,
    /// The plugin has no registration with this key.
    #[error("no file is registered with this key")]
    NotFound,
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

impl RegistryError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::AlreadyRegistered { .. } => "dbfiles.already_registered",
            Self::TooMany => "dbfiles.too_many",
            Self::NotFound => "dbfiles.not_found",
            Self::InvalidData { .. } => "dbfiles.invalid_data",
            Self::Store(error) => error.code(),
        }
    }
}

impl From<TransactError<Self>> for RegistryError {
    fn from(error: TransactError<Self>) -> Self {
        match error {
            TransactError::Rejected(error) => error,
            TransactError::Store(error) => Self::Store(error),
        }
    }
}

/// A registration as its row holds it.
struct RawRegistration {
    key: String,
    name: String,
    path: String,
    identity: String,
    read_only: i64,
    created_at: i64,
    updated_at: i64,
}

impl RawRegistration {
    fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            key: row.get(0)?,
            name: row.get(1)?,
            path: row.get(2)?,
            identity: row.get(3)?,
            read_only: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    }

    fn into_registration(self) -> Result<Registration, RegistryError> {
        let key = DatabaseKey::parse(&self.key).map_err(|error| RegistryError::InvalidData {
            what: format!("a registration's key: {error}"),
        })?;
        Ok(Registration {
            key,
            name: self.name,
            path: PathBuf::from(self.path),
            identity: self.identity,
            read_only: self.read_only != 0,
            created_at: UtcMillis::from_millis(self.created_at),
            updated_at: UtcMillis::from_millis(self.updated_at),
        })
    }
}

/// A path as the registry keeps it. Paths that the file checks return are
/// canonical, so they are valid Unicode on the systems that Mannie runs on,
/// and a path that is not is kept with replacement characters, which no
/// check then finds.
fn path_text(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

impl Store {
    /// The files that a plugin registered, by name.
    pub async fn db_files(&self, plugin: &str) -> Result<Vec<Registration>, RegistryError> {
        let plugin = plugin.to_owned();
        let rows = self
            .read(move |connection| {
                let mut select = connection.prepare(SELECT_ALL)?;
                let rows = select.query_map([plugin], RawRegistration::read)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await?;
        rows.into_iter()
            .map(RawRegistration::into_registration)
            .collect()
    }

    /// A plugin's registration with this key.
    pub async fn db_file(
        &self,
        plugin: &str,
        key: &DatabaseKey,
    ) -> Result<Registration, RegistryError> {
        let (plugin, key) = (plugin.to_owned(), key.to_string());
        let row = self
            .read(move |connection| {
                connection
                    .query_row(SELECT_ONE, params![plugin, key], RawRegistration::read)
                    .optional()
            })
            .await?;
        row.ok_or(RegistryError::NotFound)?.into_registration()
    }

    /// Registers a file for a plugin. The plugin registers each file once,
    /// and at most [`MAX_REGISTRATIONS`] files.
    pub async fn add_db_file(
        &self,
        plugin: &str,
        new: NewRegistration,
    ) -> Result<Registration, RegistryError> {
        let plugin = plugin.to_owned();
        let now = self.clock().now();
        let added = new.clone();
        self.transact(Durability::Full, move |transaction| {
            let path = path_text(&new.path);
            let existing: Option<String> = transaction
                .query_row(SELECT_PATH, params![plugin, path], |row| row.get(0))
                .optional()?;
            if let Some(key) = existing {
                let key = DatabaseKey::parse(&key).map_err(|error| {
                    TransactError::Rejected(RegistryError::InvalidData {
                        what: format!("a registration's key: {error}"),
                    })
                })?;
                return Err(TransactError::Rejected(RegistryError::AlreadyRegistered {
                    key,
                }));
            }
            let count: i64 = transaction.query_row(COUNT, [&plugin], |row| row.get(0))?;
            if usize::try_from(count).unwrap_or(usize::MAX) >= MAX_REGISTRATIONS {
                return Err(TransactError::Rejected(RegistryError::TooMany));
            }
            transaction.execute(
                INSERT,
                params![
                    plugin,
                    new.key.to_string(),
                    new.name,
                    path,
                    new.identity,
                    i64::from(new.read_only),
                    now.as_millis()
                ],
            )?;
            Ok(())
        })
        .await?;
        Ok(Registration {
            key: added.key,
            name: added.name,
            path: added.path,
            identity: added.identity,
            read_only: added.read_only,
            created_at: now,
            updated_at: now,
        })
    }

    /// Changes the owner's name for a registered file.
    pub async fn rename_db_file(
        &self,
        plugin: &str,
        key: &DatabaseKey,
        name: &str,
    ) -> Result<(), RegistryError> {
        self.change_db_file(RENAME, plugin, key, name.to_owned())
            .await
    }

    /// Marks a registered file read-only, or lifts the mark.
    pub async fn mark_db_file(
        &self,
        plugin: &str,
        key: &DatabaseKey,
        read_only: bool,
    ) -> Result<(), RegistryError> {
        self.change_db_file(MARK, plugin, key, i64::from(read_only))
            .await
    }

    /// Accepts the file that is now at a registration's path, such as after
    /// a program saved it by replacing it, with its identity from the file's
    /// checks.
    pub async fn accept_db_file(
        &self,
        plugin: &str,
        key: &DatabaseKey,
        identity: &str,
    ) -> Result<(), RegistryError> {
        self.change_db_file(IDENTIFY, plugin, key, identity.to_owned())
            .await
    }

    /// Changes one field of a registration.
    async fn change_db_file<V>(
        &self,
        statement: &'static str,
        plugin: &str,
        key: &DatabaseKey,
        value: V,
    ) -> Result<(), RegistryError>
    where
        V: rusqlite::ToSql + Send + 'static,
    {
        let (plugin, key) = (plugin.to_owned(), key.to_string());
        let now = self.clock().now().as_millis();
        let changed = self
            .write(Durability::Full, move |transaction| {
                transaction.execute(statement, params![plugin, key, value, now])
            })
            .await?;
        if changed == 0 {
            return Err(RegistryError::NotFound);
        }
        Ok(())
    }

    /// Forgets a registered file. The file itself stays as it is.
    pub async fn forget_db_file(
        &self,
        plugin: &str,
        key: &DatabaseKey,
    ) -> Result<(), RegistryError> {
        let (plugin, key) = (plugin.to_owned(), key.to_string());
        let removed = self
            .write(Durability::Full, move |transaction| {
                transaction.execute(DELETE, params![plugin, key])
            })
            .await?;
        if removed == 0 {
            return Err(RegistryError::NotFound);
        }
        Ok(())
    }
}
