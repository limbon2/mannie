//! The plugin's definition (plugins/designer/docs/README.md): its interface,
//! its table of projects, its start and stop, and the handlers of its command
//! and channel. It is off by default, like every plugin.
//!
//! A start reads the projects from the store and hands them to the channel
//! `follow_projects`, which follows each change until the plugin stops. The
//! host answers `plugin_off` while the plugin is off (ADR-0034, decision 8).
//! Which project the owner has selected belongs to the UI, which is why the
//! plugin keeps none.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures_util::StreamExt as _;
use futures_util::stream;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::StartContext;
use mannie_plugin_sdk::contracts::{Clock, UtcMillis};
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::ipc::{Handlers, Interface, PluginStream};
use mannie_plugin_sdk::plugin::{Instance, Plugin, Started};
use mannie_plugin_sdk::services::graphics::Graphics;
use mannie_plugin_sdk::services::store::{Store, StoreModule};
use tokio::sync::watch;

use crate::commands::{
    CreateProject, CreateProjectCommand, DesignerError, FollowProjectsChannel, ProjectList,
    ProjectView, interface,
};
use crate::store::{DesignerStore, MODULE, NAME_TAKEN, ProjectRecord, WriteFailure};

/// The plugin's ID.
pub const DESIGNER: &str = "designer";

/// The code of a start that could not read the projects.
pub const STORAGE_FAILED: &str = "storage_failed";

/// The plugin.
#[derive(Debug, Clone, Copy, Default)]
pub struct DesignerPlugin;

impl DesignerPlugin {
    /// The plugin, off.
    pub fn new() -> Self {
        Self
    }
}

/// The lock's guard, which a panic in another thread does not poison for long
/// enough to matter: what the plugin keeps there is a list that a start read.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A project as the UI shows it.
fn project_view(project: &ProjectRecord) -> ProjectView {
    ProjectView {
        id: project.id.clone(),
        name: project.name.clone(),
        created_at: UtcMillis::from_millis(project.created_at),
    }
}

/// The projects of a start, and what the plugin reaches of the store.
#[derive(Debug)]
pub(crate) struct Projects {
    clock: Arc<dyn Clock>,
    /// The projects, the newest first.
    list: Mutex<Vec<ProjectView>>,
    /// Counts the changes, which the channel follows.
    revisions: watch::Sender<u64>,
    store: DesignerStore,
}

impl Projects {
    pub(crate) fn store(&self) -> &DesignerStore {
        &self.store
    }

    pub(crate) fn now(&self) -> i64 {
        self.clock.now().as_millis()
    }

    /// Whether an owner-created project can open a viewport in this start.
    pub(crate) fn contains(&self, id: &str) -> bool {
        lock(&self.list).iter().any(|project| project.id == id)
    }

    /// Reads the projects that the owner created before this start.
    async fn load(store: DesignerStore, clock: Arc<dyn Clock>) -> Result<Self, WriteFailure> {
        let projects = store
            .projects()
            .await?
            .iter()
            .map(project_view)
            .collect::<Vec<_>>();
        // The sender keeps a value while no channel follows it, so the start
        // keeps the receiver only through the sender.
        let (revisions, _receiver) = watch::channel(0);
        Ok(Self {
            clock,
            list: Mutex::new(projects),
            revisions,
            store,
        })
    }

    /// Every project, the newest first.
    fn list(&self) -> ProjectList {
        ProjectList {
            projects: lock(&self.list).clone(),
        }
    }

    /// The changes of the projects, which a channel follows.
    fn follow(&self) -> watch::Receiver<u64> {
        self.revisions.subscribe()
    }

    /// Keeps a project that the owner created and hands it to the channels.
    async fn create_project(&self, name: &str) -> Result<ProjectView, DesignerError> {
        let project = ProjectRecord {
            id: uuid::Uuid::now_v7().to_string(),
            name: name.trim().to_owned(),
            created_at: self.clock.now().as_millis(),
        };

        match self.store.insert_project(&project).await {
            Ok(()) => {}
            Err(WriteFailure::Constraint(code)) if code == NAME_TAKEN => {
                return Err(DesignerError::NameTaken);
            }
            Err(failure) => {
                tracing::warn!(failure = ?failure, "the plugin designer could not keep a project");
                return Err(DesignerError::StorageFailed);
            }
        }

        let project = project_view(&project);
        lock(&self.list).insert(0, project.clone());
        self.revisions.send_modify(|revision| *revision += 1);
        tracing::info!(project = %project.id, "a project was created");
        Ok(project)
    }
}

/// The projects as they are, then after each change, until the start stops.
fn project_list(
    projects: Arc<Projects>,
    revisions: watch::Receiver<u64>,
) -> PluginStream<ProjectList> {
    stream::unfold(
        (projects, revisions, true),
        |(projects, mut revisions, first)| async move {
            if !first && revisions.changed().await.is_err() {
                return None;
            }

            revisions.borrow_and_update();
            Some((projects.list(), (projects, revisions, false)))
        },
    )
    .boxed()
}

/// The handlers of a start, which reach its projects.
fn handlers(projects: &Arc<Projects>) -> Handlers {
    let creating = Arc::clone(projects);
    let following = Arc::clone(projects);
    Handlers::new()
        .command::<CreateProjectCommand, _, _>(move |input: CreateProject| {
            let projects = Arc::clone(&creating);
            async move { projects.create_project(&input.name).await }
        })
        .channel::<FollowProjectsChannel, _, _>(move |()| {
            let revisions = following.follow();
            let projects = Arc::clone(&following);
            std::future::ready(Ok::<_, DesignerError>(project_list(projects, revisions)))
        })
}

impl DesignerPlugin {
    /// Starts the plugin on the store that the host bound to it.
    async fn start_now(context: StartContext) -> Result<Started, StartFailure> {
        let store = DesignerStore::new(context.service::<Store>()?);
        let projects = Projects::load(store, context.clock())
            .await
            .map_err(|failure| {
                tracing::error!(
                    failure = ?failure,
                    "the plugin designer could not read its projects"
                );
                StartFailure::new(STORAGE_FAILED)
            })?;

        let projects = Arc::new(projects);
        let handlers = crate::viewport::handlers(
            handlers(&projects),
            Arc::clone(&projects),
            context.service::<Graphics>()?,
        );
        tracing::info!("the plugin designer started");
        Ok(Started::new(DesignerInstance { projects }).with_handlers(handlers))
    }
}

impl Plugin for DesignerPlugin {
    fn interface(&self) -> Interface {
        interface()
    }

    fn store_module(&self) -> Option<StoreModule> {
        Some(MODULE)
    }

    fn start(&self, context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>> {
        Box::pin(Self::start_now(context))
    }
}

/// The plugin while it is on, with the projects it loaded on start.
#[derive(Debug)]
struct DesignerInstance {
    projects: Arc<Projects>,
}

impl Instance for DesignerInstance {
    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        // The host has already ended the plugin's calls and channels (ADR-0034, decision 8).
        drop(self.projects);
        tracing::info!("the plugin designer stopped");
        Box::pin(std::future::ready(()))
    }
}
