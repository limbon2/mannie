//! The registered databases as the feature knows them (plugins/workers/docs/
//! databases.md, registering a database): the service's list with the state
//! of each file when the feature last looked, which the view's channel, the
//! section, the mentions and the tools read without waiting. Every change
//! goes through the service and then lists the files again.
//!
//! A name is unique among the registered databases, ignoring case, so the
//! owner and agents name a database without its machine. The service keeps
//! registrations by key and path, and the feature keeps the names: a file
//! that the owner leaves unnamed takes its file's name, with a number after
//! it when another database has that name.

use std::sync::Arc;

use mannie_plugin_sdk::contracts::DatabaseKey;
use mannie_plugin_sdk::services::sqlite_files::{
    ChooseRequest, DatabaseFile, FileState, MAX_NAME_CHARS, SqliteFilesApi, SqliteFilesError,
};
use tokio::sync::{Mutex, watch};

use super::commands::{AddDatabase, DatabaseMachine, DatabaseView, DatabasesError};

/// The registered databases, the service that keeps them, and the list that
/// the feature last read of it.
#[derive(Debug, Clone)]
pub(crate) struct Databases {
    files: Arc<dyn SqliteFilesApi>,
    list: Arc<watch::Sender<Vec<DatabaseView>>>,
    /// Held while a database is registered or renamed, so two of them never
    /// take one name.
    naming: Arc<Mutex<()>>,
}

impl Databases {
    /// The databases that the service lists now.
    pub(crate) async fn start(files: Arc<dyn SqliteFilesApi>) -> Result<Self, SqliteFilesError> {
        let listed = files.files().await?;
        let (list, _receiver) = watch::channel(views(listed));
        Ok(Self {
            files,
            list: Arc::new(list),
            naming: Arc::new(Mutex::new(())),
        })
    }

    /// The service, for the reads and changes of a database.
    pub(crate) fn files(&self) -> &Arc<dyn SqliteFilesApi> {
        &self.files
    }

    /// The databases as the feature last listed them, by name.
    pub(crate) fn views(&self) -> Vec<DatabaseView> {
        self.list.borrow().clone()
    }

    /// The list now and after each change.
    pub(crate) fn subscribe(&self) -> watch::Receiver<Vec<DatabaseView>> {
        self.list.subscribe()
    }

    /// The database with this name, ignoring case.
    pub(crate) fn by_name(&self, name: &str) -> Option<DatabaseView> {
        let name = name.trim().to_lowercase();
        self.list
            .borrow()
            .iter()
            .find(|database| database.name.to_lowercase() == name)
            .cloned()
    }

    /// The database with this key.
    pub(crate) fn by_key(&self, key: &DatabaseKey) -> Option<DatabaseView> {
        self.list
            .borrow()
            .iter()
            .find(|database| database.key == *key)
            .cloned()
    }

    /// Lists the files again, with what the core finds at each path now.
    pub(crate) async fn refresh(&self) -> Result<(), SqliteFilesError> {
        let listed = self.files.files().await?;
        self.list.send_replace(views(listed));
        Ok(())
    }

    /// Lists the files again after a call whose error says that a file is
    /// gone or changed, so the view and the section say so; the error goes
    /// back as it is.
    pub(crate) async fn noticed(&self, error: SqliteFilesError) -> SqliteFilesError {
        let is_about_the_file = matches!(
            error,
            SqliteFilesError::FileMissing
                | SqliteFilesError::FileChanged
                | SqliteFilesError::NotRegistered
                | SqliteFilesError::NotADatabase
        );
        if is_about_the_file && let Err(refresh) = self.refresh().await {
            tracing::warn!(
                error.code = refresh.code(),
                "could not list the registered databases again"
            );
        }
        error
    }

    /// Registers the file that the owner chooses in the system's file
    /// dialog, or nothing when the owner closes it.
    pub(crate) async fn add(
        &self,
        input: AddDatabase,
    ) -> Result<Option<DatabaseView>, DatabasesError> {
        let _naming = self.naming.lock().await;
        let typed = input.name.as_deref().map(|name| name.trim().to_owned());
        if let Some(name) = &typed
            && self.name_taken(name, None)
        {
            return Err(DatabasesError::NameTaken);
        }
        let request = ChooseRequest {
            databases_filter: input.databases_filter,
            all_files_filter: input.all_files_filter,
            name: input.name,
            read_only: input.read_only,
        };
        let Some(mut file) = self.files.choose(request).await? else {
            return Ok(None);
        };
        if typed.is_none() && self.name_taken(&file.name, Some(&file.key)) {
            let unique = self.unique_name(&file.name);
            self.files.rename(file.key.clone(), unique.clone()).await?;
            file.name = unique;
        }
        self.refresh().await?;
        Ok(Some(view(file)))
    }

    /// Gives a database another name.
    pub(crate) async fn rename(&self, key: DatabaseKey, name: &str) -> Result<(), DatabasesError> {
        let _naming = self.naming.lock().await;
        if self.name_taken(name.trim(), Some(&key)) {
            return Err(DatabasesError::NameTaken);
        }
        self.files.rename(key, name.to_owned()).await?;
        self.refresh().await?;
        Ok(())
    }

    /// Marks a database read-only, or lifts the mark.
    pub(crate) async fn mark(
        &self,
        key: DatabaseKey,
        read_only: bool,
    ) -> Result<(), DatabasesError> {
        self.files.mark(key, read_only).await?;
        self.refresh().await?;
        Ok(())
    }

    /// Accepts the file that is at a database's path now.
    pub(crate) async fn accept(&self, key: DatabaseKey) -> Result<(), DatabasesError> {
        self.files.accept(key).await?;
        self.refresh().await?;
        Ok(())
    }

    /// Forgets a database and leaves its file as it is.
    pub(crate) async fn remove(&self, key: DatabaseKey) -> Result<(), DatabasesError> {
        self.files.forget(key).await?;
        self.refresh().await?;
        Ok(())
    }

    /// Whether another database than `except` has this name, ignoring case.
    fn name_taken(&self, name: &str, except: Option<&DatabaseKey>) -> bool {
        let name = name.to_lowercase();
        self.list.borrow().iter().any(|database| {
            let is_other = except != Some(&database.key);
            is_other && database.name.to_lowercase() == name
        })
    }

    /// `name` with the first number after it, from 2, that no database's name
    /// has, cut so that it keeps to the limit.
    fn unique_name(&self, name: &str) -> String {
        let mut number = 2_u32;
        loop {
            let suffix = format!(" ({number})");
            let room = MAX_NAME_CHARS.saturating_sub(suffix.chars().count());
            let base: String = name.chars().take(room).collect();
            let candidate = format!("{}{suffix}", base.trim_end());
            if !self.name_taken(&candidate, None) {
                return candidate;
            }
            number += 1;
        }
    }
}

/// The view of a registered file on this computer.
fn view(file: DatabaseFile) -> DatabaseView {
    DatabaseView {
        key: file.key,
        name: file.name,
        file_name: file.file_name,
        read_only: file.read_only,
        state: file.state,
        machine: DatabaseMachine::ThisComputer,
    }
}

/// The views of the listed files, by name.
fn views(files: Vec<DatabaseFile>) -> Vec<DatabaseView> {
    let mut views: Vec<DatabaseView> = files.into_iter().map(view).collect();
    views.sort_by_key(|database| database.name.to_lowercase());
    views
}

/// Whether a database can run statements now, by its state.
pub(crate) fn is_available(database: &DatabaseView) -> bool {
    database.state == FileState::Available
}
