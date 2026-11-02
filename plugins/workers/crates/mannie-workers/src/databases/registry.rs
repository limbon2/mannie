//! The registered databases as the feature knows them (plugins/workers/docs/
//! databases.md, registering a database): this computer's, as the service
//! lists them, and each paired worker's, as the worker listed them last,
//! which the plugin's store keeps so the view shows them while the worker is
//! offline (ADR-0057, decision 3). The view's channel, the section, the
//! mentions and the tools read the list without waiting. Every change goes
//! through the service or the worker, and the list follows it.
//!
//! The feature routes each request by its database's machine: this
//! computer's to the service, and a worker's through the link to the worker,
//! which decides it under its ceiling (decision 1).
//!
//! A name is unique among the registered databases of every machine,
//! ignoring case, so the owner and agents name a database without its
//! machine. The service and the workers keep registrations by key and path,
//! and the feature keeps the names: a file that the owner leaves unnamed
//! takes its file's name, with a number after it when another database has
//! that name.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};

use mannie_link::databases::{HostDatabase, RegisterDatabase, UpdateDatabase, WriteDone};
use mannie_link::levels;
use mannie_plugin_sdk::contracts::{CeilingMode, DatabaseKey, StatementClass, WorkerId};
use mannie_plugin_sdk::ipc::PluginError as _;
use mannie_plugin_sdk::services::sqlite_files::{
    ChangeOutcome, ChooseRequest, DatabaseFile, DatabaseStructure, DatabaseTicket, FileState,
    MAX_NAME_CHARS, OwnerChange, Page, PageRequest, ReadLimits, RemoteOutcome, SqliteFilesApi,
    SqliteFilesError, Statement, StatementOutcome, StatementRows, WorkerDatabase, WorkerHost,
};
use tokio::sync::{Mutex, watch};

use super::commands::{
    AddDatabase, AddWorkerDatabase, DatabaseMachine, DatabaseReach, DatabaseView, DatabasesError,
};
use super::remote::{Remote, RemoteError};
use crate::hub::Hub;
use crate::store::WorkersStore;
use crate::view::{WorkerStatus, WorkerView};

fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    // Every critical section only reads or replaces entries.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The error of a request on a worker's database that the worker did not
/// answer as asked. A call that failed on the way reached no worker, as far
/// as the master knows.
fn from_remote(error: RemoteError) -> DatabasesError {
    match error {
        RemoteError::Offline | RemoteError::Lost => DatabasesError::WorkerOffline,
        RemoteError::NeedsUpdate => DatabasesError::WorkerNeedsUpdate,
        RemoteError::Refused(refusal) => match refusal {
            mannie_link::databases::DatabaseRefusal::Refused { refusal } => {
                DatabasesError::HostRefused { refusal }
            }
            mannie_link::databases::DatabaseRefusal::NeedsOwner { .. } => {
                DatabasesError::NeedsOwner
            }
            mannie_link::databases::DatabaseRefusal::Files { error } => {
                DatabasesError::Files { error }
            }
        },
    }
}

/// The error of a write on a worker's database: one whose answer was lost
/// may have run.
fn from_remote_write(error: RemoteError) -> DatabasesError {
    match error {
        RemoteError::Lost => DatabasesError::AnswerLost,
        other => from_remote(other),
    }
}

/// Whether an error says that a database's file is gone or changed, after
/// which the list is read again so the view and the section say so.
fn is_about_the_file(error: &DatabasesError) -> bool {
    matches!(
        error,
        DatabasesError::Files {
            error: SqliteFilesError::FileMissing
                | SqliteFilesError::FileChanged
                | SqliteFilesError::NotRegistered
                | SqliteFilesError::NotADatabase
        } | DatabasesError::HostRefused { .. }
    )
}

/// The registered databases of every machine, the service and the hub that
/// reach them, and the list that the feature last made of them.
#[derive(Debug, Clone)]
pub(crate) struct Databases {
    files: Arc<dyn SqliteFilesApi>,
    hub: Hub,
    store: WorkersStore,
    /// This computer's registered files, as the service listed them last.
    local: Arc<StdMutex<Vec<DatabaseFile>>>,
    /// Each worker's databases, as the worker listed them last.
    remote: Arc<StdMutex<BTreeMap<WorkerId, Vec<HostDatabase>>>>,
    list: Arc<watch::Sender<Vec<DatabaseView>>>,
    /// Held while a database is registered or renamed, so two of them never
    /// take one name.
    naming: Arc<Mutex<()>>,
}

impl Databases {
    /// The databases that the service lists now and the ones that the store
    /// keeps of each worker.
    pub(crate) async fn start(
        files: Arc<dyn SqliteFilesApi>,
        hub: Hub,
    ) -> Result<Self, DatabasesError> {
        let local = files.files().await?;
        let store = hub.store();
        let kept = store.worker_databases().await.map_err(|error| {
            tracing::error!(%error, error.code = error.code(), "could not read the copy of the workers' databases");
            DatabasesError::Files {
                error: SqliteFilesError::Failed,
            }
        })?;
        let mut remote: BTreeMap<WorkerId, Vec<HostDatabase>> = BTreeMap::new();
        for (worker, database) in kept {
            remote.entry(worker).or_default().push(database);
        }
        let (list, _receiver) = watch::channel(Vec::new());
        let databases = Self {
            files,
            hub,
            store,
            local: Arc::new(StdMutex::new(local)),
            remote: Arc::new(StdMutex::new(remote)),
            list: Arc::new(list),
            naming: Arc::new(Mutex::new(())),
        };
        databases.publish();
        Ok(databases)
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

    /// Makes the list again from this computer's files, the workers'
    /// databases and what the hub knows of each worker now.
    fn publish(&self) {
        let workers: Vec<WorkerView> = self.hub.worker_views().borrow().clone();
        let mut views: Vec<DatabaseView> =
            lock(&self.local).iter().cloned().map(local_view).collect();
        for (worker, databases) in lock(&self.remote).iter() {
            let Some(shown) = workers.iter().find(|view| view.id == *worker) else {
                continue;
            };
            let reach = self.reach(shown);
            views.extend(databases.iter().map(|database| DatabaseView {
                key: database.file.key.clone(),
                name: database.file.name.clone(),
                file_name: database.file.file_name.clone(),
                read_only: database.file.read_only,
                state: database.file.state,
                machine: DatabaseMachine::Worker {
                    worker: worker.clone(),
                    name: shown.name.clone(),
                },
                reach,
            }));
        }
        views.sort_by_key(|database| database.name.to_lowercase());
        self.list.send_replace(views);
    }

    /// Whether the master reaches a worker's databases now.
    fn reach(&self, worker: &WorkerView) -> DatabaseReach {
        let Some(connection) = self.hub.connection(&worker.id) else {
            return DatabaseReach::WorkerOffline;
        };
        if !levels::speaks(&connection.levels, levels::DATABASES) {
            DatabaseReach::WorkerNeedsUpdate
        } else if worker.details.databases {
            DatabaseReach::Reachable
        } else {
            DatabaseReach::NotAllowed
        }
    }

    /// Lists this computer's files again, with what the core finds at each
    /// path now.
    async fn refresh_local(&self) -> Result<(), DatabasesError> {
        let listed = self.files.files().await?;
        *lock(&self.local) = listed;
        self.publish();
        Ok(())
    }

    /// Lists a worker's databases again and keeps the list in the store.
    async fn refresh_worker(&self, worker: &WorkerId) -> Result<(), DatabasesError> {
        let listed = Remote::of(&self.hub, worker)
            .map_err(from_remote)?
            .list()
            .await
            .map_err(from_remote)?;
        if let Err(error) = self.store.replace_worker_databases(worker, &listed).await {
            tracing::warn!(%error, error.code = error.code(), "could not keep a worker's list of its databases");
        }
        lock(&self.remote).insert(worker.clone(), listed);
        self.publish();
        Ok(())
    }

    /// Lists every machine's databases again: this computer's files and the
    /// databases of each worker that the master reaches now.
    pub(crate) async fn refresh(&self) -> Result<(), DatabasesError> {
        self.refresh_local().await?;
        let workers: Vec<WorkerId> = self
            .hub
            .worker_views()
            .borrow()
            .iter()
            .filter(|view| view.status == WorkerStatus::Connected)
            .map(|view| view.id.clone())
            .collect();
        for worker in workers {
            if let Err(error) = self.refresh_worker(&worker).await {
                tracing::debug!(
                    error.code = error.code(),
                    "could not list a worker's databases"
                );
            }
        }
        Ok(())
    }

    /// Keeps one database of a worker's list after the worker answered a
    /// change of it.
    async fn keep(&self, worker: &WorkerId, database: HostDatabase) {
        if let Err(error) = self.store.put_worker_database(worker, &database).await {
            tracing::warn!(%error, error.code = error.code(), "could not keep a worker's database");
        }
        {
            let mut remote = lock(&self.remote);
            let databases = remote.entry(worker.clone()).or_default();
            databases.retain(|kept| kept.file.key != database.file.key);
            databases.push(database);
        }
        self.publish();
    }

    /// Forgets one database of a worker's list.
    async fn drop_database(&self, worker: &WorkerId, key: &DatabaseKey) {
        if let Err(error) = self.store.remove_worker_database(worker, key).await {
            tracing::warn!(%error, error.code = error.code(), "could not forget a worker's database");
        }
        if let Some(databases) = lock(&self.remote).get_mut(worker) {
            databases.retain(|kept| kept.file.key != *key);
        }
        self.publish();
    }

    /// Follows the paired workers until the feature stops: lists a worker's
    /// databases when it connects, says when one goes offline or needs an
    /// update, and drops the databases of a worker that the owner removed,
    /// whose rows the store dropped with it.
    pub(crate) async fn follow_workers(self) {
        let mut workers = self.hub.worker_views();
        let mut connected: HashSet<WorkerId> = HashSet::new();
        loop {
            let current: Vec<WorkerView> = workers.borrow_and_update().clone();
            let now_connected: HashSet<WorkerId> = current
                .iter()
                .filter(|view| view.status == WorkerStatus::Connected)
                .map(|view| view.id.clone())
                .collect();
            lock(&self.remote)
                .retain(|worker, _databases| current.iter().any(|view| view.id == *worker));
            self.publish();
            for worker in now_connected.difference(&connected) {
                if let Err(error) = self.refresh_worker(worker).await {
                    tracing::debug!(
                        error.code = error.code(),
                        "could not list a worker's databases"
                    );
                }
            }
            connected = now_connected;
            if workers.changed().await.is_err() {
                return;
            }
        }
    }

    /// The error of a request on this computer's database, after the list
    /// is read again when the error is about the file.
    async fn noticed_here(&self, error: SqliteFilesError) -> DatabasesError {
        let error = DatabasesError::from(error);
        if is_about_the_file(&error)
            && let Err(refresh) = self.refresh_local().await
        {
            tracing::warn!(
                error.code = refresh.code(),
                "could not list the registered databases again"
            );
        }
        error
    }

    /// The error of a request on a worker's database, after the worker's
    /// list is read again when the error is about the file.
    async fn noticed_on(&self, worker: &WorkerId, error: DatabasesError) -> DatabasesError {
        if is_about_the_file(&error)
            && let Err(refresh) = self.refresh_worker(worker).await
        {
            tracing::debug!(
                error.code = refresh.code(),
                "could not list a worker's databases again"
            );
        }
        error
    }

    /// The machine of the database with this key.
    fn machine(&self, key: &DatabaseKey) -> Result<DatabaseMachine, DatabasesError> {
        self.by_key(key)
            .map(|database| database.machine)
            .ok_or(DatabasesError::Files {
                error: SqliteFilesError::NotRegistered,
            })
    }

    /// The client of a worker's databases.
    fn remote(&self, worker: &WorkerId) -> Result<Remote, DatabasesError> {
        Remote::of(&self.hub, worker).map_err(from_remote)
    }

    /// Registers the file that the owner chooses in this computer's file
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
        self.refresh_local().await?;
        Ok(Some(local_view(file)))
    }

    /// Registers the file at a path that the owner typed on a worker's host,
    /// which the worker checks (ADR-0057, decision 3).
    pub(crate) async fn add_on_worker(
        &self,
        input: AddWorkerDatabase,
    ) -> Result<DatabaseView, DatabasesError> {
        let _naming = self.naming.lock().await;
        let typed = input.name.as_deref().map(|name| name.trim().to_owned());
        if let Some(name) = &typed
            && self.name_taken(name, None)
        {
            return Err(DatabasesError::NameTaken);
        }
        let register = RegisterDatabase {
            path: input.path,
            name: input.name,
            read_only: input.read_only,
        };
        let mut registered = self
            .remote(&input.worker)?
            .register(&register)
            .await
            .map_err(from_remote)?;
        if typed.is_none() && self.name_taken(&registered.file.name, Some(&registered.file.key)) {
            let unique = self.unique_name(&registered.file.name);
            registered = self
                .remote(&input.worker)?
                .update(
                    &registered.file.key,
                    &UpdateDatabase::Rename { name: unique },
                )
                .await
                .map_err(from_remote)?;
        }
        let key = registered.file.key.clone();
        self.keep(&input.worker, registered).await;
        self.by_key(&key).ok_or(DatabasesError::Files {
            error: SqliteFilesError::NotRegistered,
        })
    }

    /// Changes a worker's database and keeps the worker's answer.
    async fn update_on_worker(
        &self,
        worker: &WorkerId,
        key: &DatabaseKey,
        update: &UpdateDatabase,
    ) -> Result<(), DatabasesError> {
        let updated = match self.remote(worker)?.update(key, update).await {
            Ok(updated) => updated,
            Err(error) => return Err(self.noticed_on(worker, from_remote(error)).await),
        };
        self.keep(worker, updated).await;
        Ok(())
    }

    /// Gives a database another name.
    pub(crate) async fn rename(&self, key: DatabaseKey, name: &str) -> Result<(), DatabasesError> {
        let _naming = self.naming.lock().await;
        if self.name_taken(name.trim(), Some(&key)) {
            return Err(DatabasesError::NameTaken);
        }
        match self.machine(&key)? {
            DatabaseMachine::ThisComputer => {
                self.files.rename(key, name.to_owned()).await?;
                self.refresh_local().await
            }
            DatabaseMachine::Worker { worker, .. } => {
                let update = UpdateDatabase::Rename {
                    name: name.to_owned(),
                };
                self.update_on_worker(&worker, &key, &update).await
            }
        }
    }

    /// Marks a database read-only, or lifts the mark.
    pub(crate) async fn mark(
        &self,
        key: DatabaseKey,
        read_only: bool,
    ) -> Result<(), DatabasesError> {
        match self.machine(&key)? {
            DatabaseMachine::ThisComputer => {
                self.files.mark(key, read_only).await?;
                self.refresh_local().await
            }
            DatabaseMachine::Worker { worker, .. } => {
                let update = UpdateDatabase::Mark { read_only };
                self.update_on_worker(&worker, &key, &update).await
            }
        }
    }

    /// Accepts the file that is at a database's path now.
    pub(crate) async fn accept(&self, key: DatabaseKey) -> Result<(), DatabasesError> {
        match self.machine(&key)? {
            DatabaseMachine::ThisComputer => {
                if let Err(error) = self.files.accept(key).await {
                    return Err(self.noticed_here(error).await);
                }
                self.refresh_local().await
            }
            DatabaseMachine::Worker { worker, .. } => {
                self.update_on_worker(&worker, &key, &UpdateDatabase::Accept)
                    .await
            }
        }
    }

    /// Forgets a database and leaves its file as it is.
    pub(crate) async fn remove(&self, key: DatabaseKey) -> Result<(), DatabasesError> {
        match self.machine(&key)? {
            DatabaseMachine::ThisComputer => {
                self.files.forget(key).await?;
                self.refresh_local().await
            }
            DatabaseMachine::Worker { worker, .. } => {
                if let Err(error) = self.remote(&worker)?.forget(&key).await {
                    return Err(self.noticed_on(&worker, from_remote(error)).await);
                }
                self.drop_database(&worker, &key).await;
                Ok(())
            }
        }
    }

    /// A database's structure.
    pub(crate) async fn structure(
        &self,
        key: DatabaseKey,
    ) -> Result<DatabaseStructure, DatabasesError> {
        match self.machine(&key)? {
            DatabaseMachine::ThisComputer => match self.files.structure(key).await {
                Ok(structure) => Ok(structure),
                Err(error) => Err(self.noticed_here(error).await),
            },
            DatabaseMachine::Worker { worker, .. } => {
                match self.remote(&worker)?.structure(&key).await {
                    Ok(structure) => Ok(structure),
                    Err(error) => Err(self.noticed_on(&worker, from_remote(error)).await),
                }
            }
        }
    }

    /// Counts the rows of a table or a view.
    pub(crate) async fn count(
        &self,
        key: DatabaseKey,
        table: String,
    ) -> Result<u64, DatabasesError> {
        match self.machine(&key)? {
            DatabaseMachine::ThisComputer => match self.files.count(key, table).await {
                Ok(rows) => Ok(rows),
                Err(error) => Err(self.noticed_here(error).await),
            },
            DatabaseMachine::Worker { worker, .. } => {
                match self.remote(&worker)?.count(&key, table).await {
                    Ok(rows) => Ok(rows),
                    Err(error) => Err(self.noticed_on(&worker, from_remote(error)).await),
                }
            }
        }
    }

    /// One page of a table.
    pub(crate) async fn page(
        &self,
        key: DatabaseKey,
        request: PageRequest,
    ) -> Result<Page, DatabasesError> {
        match self.machine(&key)? {
            DatabaseMachine::ThisComputer => match self.files.page(key, request).await {
                Ok(page) => Ok(page),
                Err(error) => Err(self.noticed_here(error).await),
            },
            DatabaseMachine::Worker { worker, .. } => {
                match self.remote(&worker)?.page(&key, &request).await {
                    Ok(page) => Ok(page),
                    Err(error) => Err(self.noticed_on(&worker, from_remote(error)).await),
                }
            }
        }
    }

    /// Runs one statement that changes nothing. Dropping the future stops
    /// it, on this computer and on a worker.
    pub(crate) async fn read(
        &self,
        key: DatabaseKey,
        statement: Statement,
        limits: ReadLimits,
    ) -> Result<StatementRows, DatabasesError> {
        match self.machine(&key)? {
            DatabaseMachine::ThisComputer => match self.files.read(key, statement, limits).await {
                Ok(rows) => Ok(rows),
                Err(error) => Err(self.noticed_here(error).await),
            },
            DatabaseMachine::Worker { worker, .. } => {
                match self.remote(&worker)?.read(&key, &statement, limits).await {
                    Ok(rows) => Ok(rows),
                    Err(error) => Err(self.noticed_on(&worker, from_remote(error)).await),
                }
            }
        }
    }

    /// What a statement does, read from SQLite without running it: on this
    /// computer against the file, and for a worker's database against the
    /// copy of the schema that the worker reports now (ADR-0056,
    /// decision 6).
    pub(crate) async fn classify(
        &self,
        key: DatabaseKey,
        text: String,
    ) -> Result<StatementClass, DatabasesError> {
        match self.machine(&key)? {
            DatabaseMachine::ThisComputer => match self.files.classify(key, text).await {
                Ok(class) => Ok(class),
                Err(error) => Err(self.noticed_here(error).await),
            },
            DatabaseMachine::Worker { .. } => {
                let structure = self.structure(key).await?;
                Ok(self.files.classify_copy(structure.schema(), text).await?)
            }
        }
    }

    /// What a worker reported of one of its databases, for a tool's access
    /// or the owner's change, or `None` for a database on this computer:
    /// the host's mode and its acceptance of the master's full access as
    /// its last `Hello` said, the registration as its last list said, and
    /// the copy of the schema as its structure says now (ADR-0057,
    /// decision 6).
    pub(crate) async fn worker_database(
        &self,
        key: &DatabaseKey,
    ) -> Result<Option<(WorkerId, WorkerDatabase)>, DatabasesError> {
        let database = self.by_key(key).ok_or(DatabasesError::Files {
            error: SqliteFilesError::NotRegistered,
        })?;
        let DatabaseMachine::Worker { worker, name } = database.machine else {
            return Ok(None);
        };
        let structure = self.structure(key.clone()).await?;
        let details = self
            .hub
            .worker_views()
            .borrow()
            .iter()
            .find(|view| view.id == worker)
            .map(|view| view.details.clone())
            .ok_or(DatabasesError::WorkerOffline)?;
        // A worker that reported no ceiling is taken as the strictest host
        // that still writes: it asks and refuses the master's full access.
        let (mode, accepts_full_access) = details
            .ceiling
            .map_or((CeilingMode::Ask, false), |ceiling| {
                (ceiling.mode, ceiling.accepts_master_full_access)
            });
        let identity = lock(&self.remote)
            .get(&worker)
            .and_then(|databases| {
                databases
                    .iter()
                    .find(|kept| kept.file.key == *key)
                    .map(|kept| kept.identity.clone())
            })
            .ok_or(DatabasesError::Files {
                error: SqliteFilesError::NotRegistered,
            })?;
        let reported = WorkerDatabase::new(
            WorkerHost::new(worker.clone(), name, mode, accepts_full_access),
            database.name,
            identity,
            database.read_only,
            structure.schema(),
        );
        Ok(Some((worker, reported)))
    }

    /// Runs the owner's change as an owner operation: the service runs a
    /// change on this computer, and for a worker's database the core
    /// journals it, the worker runs it with its evidence, and the journal
    /// records how it ended, uncertain when the worker's answer was lost
    /// (ADR-0057, decision 8).
    pub(crate) async fn change(
        &self,
        key: DatabaseKey,
        change: OwnerChange,
    ) -> Result<ChangeOutcome, DatabasesError> {
        let Some((worker, database)) = self.worker_database(&key).await? else {
            return match self.files.change(key, change).await {
                Ok(outcome) => Ok(outcome),
                Err(error) => Err(self.noticed_here(error).await),
            };
        };
        let remote = self.remote(&worker)?;
        let journaled = self
            .files
            .begin_remote_change(key, database, change)
            .await?;
        match remote.write(journaled.evidence()).await {
            Ok(WriteDone::Change { outcome }) => {
                journaled
                    .end(RemoteOutcome::Completed {
                        changes: outcome.changes,
                    })
                    .await;
                Ok(outcome)
            }
            Ok(WriteDone::Statements { .. }) => {
                // A worker answers a change with what the change did; an
                // answer of another kind says nothing of it.
                journaled.end(RemoteOutcome::Uncertain).await;
                Err(DatabasesError::AnswerLost)
            }
            Err(error) => {
                let error = from_remote_write(error);
                let outcome = match &error {
                    DatabasesError::AnswerLost => RemoteOutcome::Uncertain,
                    other => RemoteOutcome::Failed {
                        code: other.code().to_owned(),
                    },
                };
                journaled.end(outcome).await;
                Err(self.noticed_on(&worker, error).await)
            }
        }
    }

    /// Runs the writes that a tool's call's ticket allowed on one database:
    /// the service runs them on this computer, and a worker runs them with
    /// the evidence that the core gives (ADR-0057, decision 7).
    pub(crate) async fn write(
        &self,
        ticket: DatabaseTicket,
        key: DatabaseKey,
        statements: Vec<Statement>,
    ) -> Result<Vec<StatementOutcome>, DatabasesError> {
        let worker = match self.machine(&key)? {
            DatabaseMachine::ThisComputer => {
                return match self.files.write(ticket, key, statements).await {
                    Ok(outcomes) => Ok(outcomes),
                    Err(error) => Err(self.noticed_here(error).await),
                };
            }
            DatabaseMachine::Worker { worker, .. } => worker,
        };
        let remote = self.remote(&worker)?;
        let evidence = self.files.evidence(ticket, key, statements).await?;
        match remote.write(&evidence).await {
            Ok(WriteDone::Statements { outcomes }) => Ok(outcomes),
            Ok(WriteDone::Change { .. }) => Err(DatabasesError::AnswerLost),
            Err(error) => Err(self.noticed_on(&worker, from_remote_write(error)).await),
        }
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
fn local_view(file: DatabaseFile) -> DatabaseView {
    DatabaseView {
        key: file.key,
        name: file.name,
        file_name: file.file_name,
        read_only: file.read_only,
        state: file.state,
        machine: DatabaseMachine::ThisComputer,
        reach: DatabaseReach::Reachable,
    }
}

/// Whether a database can run statements now: its file is there and its
/// machine is reached.
pub(crate) fn is_available(database: &DatabaseView) -> bool {
    database.state == FileState::Available && database.reach == DatabaseReach::Reachable
}
