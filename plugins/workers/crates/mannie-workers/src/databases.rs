//! The feature Databases of Workers (plugins/workers/docs/databases.md;
//! ADR-0055, ADR-0056, ADR-0057): the owner's SQLite files on this computer
//! and on the paired workers' hosts, which its view browses and edits and
//! agents query and change. The owner turns it on under Workers on the
//! Plugins page; while it is off, Workers works as before and the core holds
//! no binding of the service `sqlite-files`.
//!
//! A start lists the registered files, reads the copy of each worker's list
//! and follows the workers, and contributes the three tools, the section of
//! the databases and the mention provider, with the handlers of the view's
//! commands and channel. Turning the feature off removes them, ends its
//! calls and channel with `feature_off`, waits for its work, stops following
//! the workers and releases the service; the registrations stay.

pub mod commands;
mod mentions;
mod output;
mod registry;
mod remote;
mod section;
mod texts;
pub mod tools;

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures_util::future::{AbortHandle, abortable};
use futures_util::{StreamExt as _, stream};
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::FeatureContext;
use mannie_plugin_sdk::contracts::WorkItem;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::ipc::{Handlers, PluginError as _, PluginStream};
use mannie_plugin_sdk::plugin::{FeatureInstance, FeatureStarted};
use mannie_plugin_sdk::points::mentions::{MentionProvider, Mentions};
use mannie_plugin_sdk::points::sections::{Section, SectionBody, Sections};
use mannie_plugin_sdk::points::tools::{AgentSelector, PluginTool, Tools};
use mannie_plugin_sdk::services::sqlite_files::{ReadLimits, SqliteFiles, StatementRows};
use tokio::sync::watch;
use tokio::task::JoinHandle;

pub use self::commands::DATABASES;
use self::commands::{
    AcceptDatabaseCommand, AddDatabase, AddDatabaseCommand, AddWorkerDatabase,
    AddWorkerDatabaseCommand, ChangeDatabase, ChangeDatabaseCommand, ChangeDone, ClassifyStatement,
    ClassifyStatementCommand, CountRows, CountRowsCommand, DatabaseListUpdate, DatabasePageCommand,
    DatabaseRef, DatabaseStructureCommand, DatabaseView, DatabasesError, FollowDatabasesChannel,
    MarkDatabase, MarkDatabaseCommand, PageDatabase, ReadDatabase, ReadDatabaseCommand,
    RefreshDatabasesCommand, RemoveDatabaseCommand, RenameDatabase, RenameDatabaseCommand,
    StopRead, StopReadCommand,
};
use self::mentions::DatabaseMentions;
use self::registry::Databases;
use self::section::{DatabasesSection, SECTION_ID, SECTION_POSITION};
use self::tools::{
    CHANGING_AGENTS, ChangeDatabase as ChangeTool, DescribeDatabase, QueryDatabase, READING_AGENTS,
};
use crate::hub::Hub;

/// The code of a start that could not list the registered databases.
pub const DATABASES_UNAVAILABLE: &str = "databases_unavailable";

/// The code of the view's statements that run, which turning the feature off
/// lets go.
pub const RUNNING_STATEMENTS: &str = "running_statements";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every critical section only adds, takes or reads entries.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Starts the feature with the service that it declares and the hub of
/// the plugin's start, which reaches the workers.
pub(crate) fn start(
    context: FeatureContext,
    hub: Hub,
) -> BoxFuture<'static, Result<FeatureStarted, StartFailure>> {
    Box::pin(async move {
        let files = context.service::<SqliteFiles>()?;
        let databases = Databases::start(files, hub).await.map_err(|error| {
            tracing::error!(
                error.code = error.code(),
                "the feature Databases could not list the registered databases"
            );
            StartFailure::new(DATABASES_UNAVAILABLE)
        })?;
        let running = Running::default();
        let contributions = contributions(&databases);
        let handlers = handlers(&databases, &running);
        let follower = tokio::spawn(databases.follow_workers());
        tracing::info!("the feature Databases of Workers started");
        Ok(FeatureStarted::new(DatabasesInstance { running, follower })
            .with_contributions(contributions)
            .with_handlers(handlers))
    })
}

/// The tools, the section and the mention provider of a start.
fn contributions(databases: &Databases) -> Contributions {
    let agents = |names: &[&str]| -> AgentSelector {
        AgentSelector::Agents(names.iter().map(|name| (*name).to_owned()).collect())
    };
    let describe = PluginTool::new(
        Arc::new(DescribeDatabase::new(databases.clone())),
        agents(&READING_AGENTS),
    );
    let query = PluginTool::new(
        Arc::new(QueryDatabase::new(databases.clone())),
        agents(&READING_AGENTS),
    )
    .with_database_reads();
    let change = PluginTool::new(
        Arc::new(ChangeTool::new(databases.clone())),
        agents(&CHANGING_AGENTS),
    )
    .with_side_effects();
    let section = Section::new(
        SECTION_ID,
        SECTION_POSITION,
        SectionBody::Source(Arc::new(DatabasesSection::new(databases.clone()))),
    );
    let mentions: Arc<dyn MentionProvider> = Arc::new(DatabaseMentions::new(databases.clone()));
    Contributions::new()
        .add::<Tools>(describe)
        .add::<Tools>(query)
        .add::<Tools>(change)
        .add::<Sections>(section)
        .add::<Mentions>(mentions)
}

/// The view's statements that run: the reads that it may stop, by the IDs
/// that it gave them, and how many reads and changes run.
#[derive(Debug, Clone, Default)]
struct Running {
    reads: Arc<Mutex<HashMap<String, AbortHandle>>>,
    count: Arc<AtomicU32>,
}

/// One running statement of the view, counted while it lives.
struct Counted(Arc<AtomicU32>);

impl Counted {
    fn new(count: &Arc<AtomicU32>) -> Self {
        count.fetch_add(1, Ordering::Relaxed);
        Self(Arc::clone(count))
    }
}

impl Drop for Counted {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Running {
    /// Runs a read of the view, which `stop` ends when it has an ID.
    async fn read(
        &self,
        databases: Databases,
        input: ReadDatabase,
    ) -> Result<StatementRows, DatabasesError> {
        let _counted = Counted::new(&self.count);
        let (reading, handle) =
            abortable(databases.read(input.key, input.statement, ReadLimits::VIEW));
        if let Some(run) = &input.run {
            lock(&self.reads).insert(run.clone(), handle);
        }
        let read = reading.await;
        if let Some(run) = &input.run {
            lock(&self.reads).remove(run);
        }
        match read {
            Ok(read) => read,
            // Dropping the read stops its statement in the engine, on this
            // computer or on the worker, which sees its call cancelled.
            Err(_aborted) => Err(DatabasesError::Stopped),
        }
    }

    /// Stops the read with this ID; one that ended stays as it is.
    fn stop(&self, run: &str) {
        let handle = lock(&self.reads).remove(run);
        if let Some(handle) = handle {
            handle.abort();
        }
    }

    /// Runs an owner's change, which the core journals before it runs.
    async fn change(
        &self,
        databases: Databases,
        input: ChangeDatabase,
    ) -> Result<ChangeDone, DatabasesError> {
        let _counted = Counted::new(&self.count);
        let outcome = databases.change(input.key, input.change).await?;
        Ok(ChangeDone {
            changes: outcome.changes.to_string(),
            rowid: outcome.rowid.map(|rowid| rowid.to_string()),
            rows: outcome.rows,
        })
    }
}

/// The databases as they are, then after each change, until the start
/// stops.
fn database_list(list: watch::Receiver<Vec<DatabaseView>>) -> PluginStream<DatabaseListUpdate> {
    stream::unfold((list, true), |(mut list, first)| async move {
        if !first && list.changed().await.is_err() {
            return None;
        }
        let current = list.borrow_and_update().clone();
        Some((
            DatabaseListUpdate::Databases { databases: current },
            (list, false),
        ))
    })
    .boxed()
}

/// The handlers of the registrations and the channel of a start.
fn registration_handlers(databases: &Databases) -> Handlers {
    let (refreshing, adding, renaming) = (databases.clone(), databases.clone(), databases.clone());
    let (marking, accepting, removing) = (databases.clone(), databases.clone(), databases.clone());
    let (following, adding_on_worker) = (databases.clone(), databases.clone());
    Handlers::new()
        .command::<RefreshDatabasesCommand, _, _>(move |()| {
            let databases = refreshing.clone();
            async move { databases.refresh().await }
        })
        .command::<AddDatabaseCommand, _, _>(move |input: AddDatabase| {
            let databases = adding.clone();
            async move { databases.add(input).await }
        })
        .command::<AddWorkerDatabaseCommand, _, _>(move |input: AddWorkerDatabase| {
            let databases = adding_on_worker.clone();
            async move { databases.add_on_worker(input).await }
        })
        .command::<RenameDatabaseCommand, _, _>(move |input: RenameDatabase| {
            let databases = renaming.clone();
            async move { databases.rename(input.key, &input.name).await }
        })
        .command::<MarkDatabaseCommand, _, _>(move |input: MarkDatabase| {
            let databases = marking.clone();
            async move { databases.mark(input.key, input.read_only).await }
        })
        .command::<AcceptDatabaseCommand, _, _>(move |input: DatabaseRef| {
            let databases = accepting.clone();
            async move { databases.accept(input.key).await }
        })
        .command::<RemoveDatabaseCommand, _, _>(move |input: DatabaseRef| {
            let databases = removing.clone();
            async move { databases.remove(input.key).await }
        })
        .channel::<FollowDatabasesChannel, _, _>(move |()| {
            let list = database_list(following.subscribe());
            std::future::ready(Ok::<_, DatabasesError>(list))
        })
}

/// The handlers of a start: the registrations, the channel, and the reads
/// and changes of the view.
fn handlers(databases: &Databases, running: &Running) -> Handlers {
    let (structuring, counting, paging) = (databases.clone(), databases.clone(), databases.clone());
    let (classifying, reading, changing) =
        (databases.clone(), databases.clone(), databases.clone());
    let (read_runs, stop_runs, change_runs) = (running.clone(), running.clone(), running.clone());
    registration_handlers(databases)
        .command::<DatabaseStructureCommand, _, _>(move |input: DatabaseRef| {
            let databases = structuring.clone();
            async move { databases.structure(input.key).await }
        })
        .command::<CountRowsCommand, _, _>(move |input: CountRows| {
            let databases = counting.clone();
            async move {
                let rows = databases.count(input.key, input.table).await?;
                Ok(rows.to_string())
            }
        })
        .command::<DatabasePageCommand, _, _>(move |input: PageDatabase| {
            let databases = paging.clone();
            async move { databases.page(input.key, input.request).await }
        })
        .command::<ClassifyStatementCommand, _, _>(move |input: ClassifyStatement| {
            let databases = classifying.clone();
            async move { databases.classify(input.key, input.text).await }
        })
        .command::<ReadDatabaseCommand, _, _>(move |input: ReadDatabase| {
            let (running, databases) = (read_runs.clone(), reading.clone());
            async move { running.read(databases, input).await }
        })
        .command::<StopReadCommand, _, _>(move |input: StopRead| {
            stop_runs.stop(&input.run);
            std::future::ready(Ok::<_, DatabasesError>(()))
        })
        .command::<ChangeDatabaseCommand, _, _>(move |input: ChangeDatabase| {
            let (running, databases) = (change_runs.clone(), changing.clone());
            async move { running.change(databases, input).await }
        })
}

/// Databases while it is on: the view's statements that run, and the task
/// that follows the workers.
#[derive(Debug)]
struct DatabasesInstance {
    running: Running,
    follower: JoinHandle<()>,
}

impl FeatureInstance for DatabasesInstance {
    /// The view's reads and changes that run, which turning the feature off
    /// lets go: a read stops, and a change runs to its end and its record.
    fn work(&self) -> BoxFuture<'static, Vec<WorkItem>> {
        let count = self.running.count.load(Ordering::Relaxed);
        let work = if count == 0 {
            Vec::new()
        } else {
            vec![WorkItem::new(RUNNING_STATEMENTS, count)]
        };
        Box::pin(std::future::ready(work))
    }

    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        let handles: Vec<AbortHandle> = lock(&self.running.reads)
            .drain()
            .map(|(_run, handle)| handle)
            .collect();
        for handle in handles {
            handle.abort();
        }
        self.follower.abort();
        tracing::info!("the feature Databases of Workers stopped");
        Box::pin(std::future::ready(()))
    }
}
