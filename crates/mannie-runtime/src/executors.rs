//! Task executors (ADR-0031, decision 4): the runtime's side of the
//! executors that plugins contribute to run the tasks of a reply somewhere
//! else, such as on a worker. The contract that a plugin implements is the
//! SDK's `points::executors` (ADR-0034, decision 6), whose consumer is the
//! runtime's registry; this module checks a new executor's kind, finds the
//! executor of a kind, and follows the executors' tasks.
//!
//! Child agents are the runtime's own executor. An executor's task is a task
//! of ADR-0024 all the same: a `task` part with a number among the
//! conversation's children, on the run's board and in its lane, with
//! background tasks, follow-ups and waits. The executor submits, follows and
//! cancels its tasks, keeps what it needs in a document on the task's row,
//! and publishes each state through [`Runtime::publish_task_state`]; the
//! runtime stores the state in the task's row and shows it on the board of
//! the run that follows the task.
//!
//! The runtime provides two services of the SDK (ADR-0034, decision 5): the
//! executor host, with the rows, states, documents and outbox of a plugin's
//! tasks, and credentials, which lends a provider credential for one of its
//! tasks. Each binds to the executor kind that equals the plugin's ID, so a
//! plugin reaches no other kind's tasks, and a lend needs an active task of
//! that kind whose run uses the profile, as the store records it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_plugin_sdk::contributions::{Contributions, ExtensionPoint};
use mannie_plugin_sdk::host::{
    BindRefusal, ContributionRefusal, ID_TAKEN, PluginBinding, ServiceProvider,
};
use mannie_plugin_sdk::points::executors::{Executors, TaskExecutor};
use mannie_plugin_sdk::service::{BoundService, Service};
use mannie_plugin_sdk::services::credentials::{self, Credentials, CredentialsApi};
use mannie_plugin_sdk::services::executor_host::{
    ExecutorHost, ExecutorHostApi, ExecutorTask, HostFailure, OutboxItem,
};
use mannie_store::{OutboxEntry, RuntimeStoreError, StoredTask, TaskEnding};
use mannie_types::chat::ChatError;
use mannie_types::ids::{ConversationId, ProfileId, TaskId};
use mannie_types::tasks::{AssignedTaskState, TaskConversation, TaskExecutorRef, TaskState};
use mannie_types::tools::ToolsError;
use mannie_types::transcript::ConversationSummary;
use tokio::sync::watch;

use crate::Runtime;
use crate::plugins::Snapshot;
use crate::profiles::{VaultError, in_vault};
use crate::tasks::{ChildEnd, TaskBoard};

/// The board of a run that follows an executor's task.
#[derive(Debug)]
struct FollowingBoard {
    /// The kind of the executor that runs the task.
    kind: String,
    board: Arc<TaskBoard>,
}

/// The boards of the runs that follow the executors' tasks, and the changes
/// of the outbox that the executors take deletions from. The executors
/// themselves are in the runtime's registry of contributions.
#[derive(Debug)]
pub(crate) struct ExecutorFollows {
    /// The boards of the runs that follow executors' tasks, by task.
    boards: Mutex<HashMap<TaskId, FollowingBoard>>,
    /// Counts the deletions that wait in the outbox, so each executor takes
    /// them while its plugin is on.
    outbox: watch::Sender<u64>,
}

impl Default for ExecutorFollows {
    fn default() -> Self {
        Self {
            boards: Mutex::default(),
            outbox: watch::Sender::new(0),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every critical section only inserts or removes entries, so the data
    // stays consistent even if a holder panicked.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ExecutorFollows {
    fn board(&self, task: &TaskId) -> Option<Arc<TaskBoard>> {
        lock(&self.boards)
            .get(task)
            .map(|following| Arc::clone(&following.board))
    }

    /// Tells each run that follows a task of `kind` to let the task go,
    /// because the executor's plugin stopped.
    pub(crate) fn let_go_of(&self, kind: &str) -> usize {
        let boards = lock(&self.boards);
        let mut count = 0;
        for (task, following) in boards
            .iter()
            .filter(|(_task, following)| following.kind == kind)
        {
            following.board.publish(task, |state| state.let_go = true);
            count += 1;
        }
        count
    }

    /// Wakes the executors to take what waits in the outbox.
    pub(crate) fn notify_outbox(&self) {
        self.outbox
            .send_modify(|count| *count = count.wrapping_add(1));
    }
}

/// A run's hold on the states of an executor's task: while it lives, the
/// states that the executor publishes reach the run's board.
#[derive(Debug)]
pub(crate) struct RemoteFollow {
    runtime: Runtime,
    task: TaskId,
}

impl RemoteFollow {
    /// Follows a task of the executor of `kind` on `board` from now on.
    pub(crate) fn new(
        runtime: &Runtime,
        task: &TaskId,
        kind: &str,
        board: &Arc<TaskBoard>,
    ) -> Self {
        let following = FollowingBoard {
            kind: kind.to_owned(),
            board: Arc::clone(board),
        };
        lock(&runtime.inner.executors.boards).insert(task.clone(), following);
        Self {
            runtime: runtime.clone(),
            task: task.clone(),
        }
    }
}

impl Drop for RemoteFollow {
    fn drop(&mut self) {
        lock(&self.runtime.inner.executors.boards).remove(&self.task);
    }
}

/// A task conversation that an executor serves.
#[derive(Debug)]
pub(crate) struct RemoteConversation {
    /// The executor, or the kind of an executor whose plugin is off.
    pub(crate) executor: Result<Arc<dyn TaskExecutor>, String>,
    /// The conversation's target.
    pub(crate) target: TaskExecutorRef,
    /// This machine's summary of the conversation.
    pub(crate) summary: ConversationSummary,
}

impl RemoteConversation {
    /// The executor, or the chat error of a plugin that is off.
    pub(crate) fn chat_executor(&self) -> Result<&Arc<dyn TaskExecutor>, ChatError> {
        self.executor.as_ref().map_err(|kind| ChatError::PluginOff {
            plugin: kind.clone(),
        })
    }

    /// The executor, or the tools error of a plugin that is off.
    pub(crate) fn tools_executor(&self) -> Result<&Arc<dyn TaskExecutor>, ToolsError> {
        self.executor
            .as_ref()
            .map_err(|kind| ToolsError::PluginOff {
                plugin: kind.clone(),
            })
    }
}

/// The ending of a task from its state, once it ended.
fn ending_of(state: &AssignedTaskState) -> Option<TaskEnding> {
    (!state.state.is_active()).then(|| TaskEnding {
        state: state.state,
        report: state.report.clone(),
        failure: state.failure.clone(),
    })
}

/// Refuses a plugin's executor whose kind another executor has. The rule
/// `PluginKind`, which the SDK's coordinator checks, and the registry's
/// refusal of a plugin that is registered already keep kinds apart; this
/// module keeps the invariant itself, because runs find an executor by its
/// kind.
pub(crate) fn check_executors(
    snapshot: &Snapshot,
    contributions: &Contributions,
) -> Result<(), ContributionRefusal> {
    for executor in contributions.entries::<Executors>() {
        if snapshot.find::<Executors>(executor.kind()).is_some() {
            return Err(ContributionRefusal::new(
                Executors::NAME,
                ID_TAKEN,
                format!(
                    "an executor of the kind {} is registered already",
                    executor.kind()
                ),
            ));
        }
    }
    Ok(())
}

impl Runtime {
    /// The executor of a kind, while its plugin is on. A run reads it anew
    /// for each call of an executor's tool and for each task, so a call goes
    /// to the executor of the plugin's latest start, and a task of a plugin
    /// that stopped is let go (ADR-0031, decision 10).
    pub(crate) fn executor(&self, kind: &str) -> Option<Arc<dyn TaskExecutor>> {
        self.contributions()
            .find::<Executors>(kind)
            .map(|registered| Arc::clone(&registered.entry))
    }

    /// The executor's task conversation that `conversation` is, with this
    /// machine's summary of it; `None` for every other conversation.
    pub(crate) async fn remote_conversation(
        &self,
        conversation: &ConversationId,
    ) -> Result<Option<RemoteConversation>, RuntimeStoreError> {
        let summary = self.inner.store.conversation_summary(conversation).await?;
        Ok(summary.and_then(|summary| {
            let target = summary.task.as_ref()?.executor.clone()?;
            let executor = self
                .executor(&target.kind)
                .ok_or_else(|| target.kind.clone());
            Some(RemoteConversation {
                executor,
                target,
                summary,
            })
        }))
    }

    /// Cancels the active task of an executor's task conversation, as Stop
    /// in the conversation does. Returns whether a task was active.
    pub(crate) async fn stop_remote_task(&self, conversation: &ConversationId) -> bool {
        let latest = match self.inner.store.latest_task(conversation).await {
            Ok(latest) => latest,
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not read the task of a conversation");
                return false;
            }
        };
        let Some(task) = latest.filter(|task| task.state.is_active()) else {
            return false;
        };
        let Some(execution) = &task.executor else {
            return false;
        };
        let Some(executor) = self.executor(&execution.kind) else {
            tracing::info!(task = %task.id, kind = %execution.kind, "a stop found the plugin of a task off");
            return false;
        };
        executor.cancel(&task.id).await;
        tracing::info!(task = %task.id, kind = %execution.kind, "the owner stopped a task of an executor");
        true
    }

    /// Publishes a state of an executor's task (ADR-0031, decision 4): the
    /// runtime stores it in the task's row and shows it on the board of the
    /// run that follows the task. A state of a task that is another
    /// executor's or another conversation's, of a task that ended, or older
    /// than the one stored changes nothing.
    ///
    /// Returns the task's row as it was before the state, for a state that
    /// the runtime took.
    pub(crate) async fn publish_task_state(
        &self,
        kind: &str,
        state: AssignedTaskState,
    ) -> Result<Option<StoredTask>, RuntimeStoreError> {
        let store = &self.inner.store;
        let Some(row) = store.task(&state.task_id).await? else {
            return Ok(None);
        };
        let Some(execution) = row.executor.as_ref() else {
            tracing::warn!(task = %state.task_id, kind, "an executor published the state of a task that is not its own");
            return Ok(None);
        };
        if execution.kind != kind || row.conversation_id != state.conversation_id {
            tracing::warn!(task = %state.task_id, kind, "an executor published the state of a task that is not its own");
            return Ok(None);
        }
        if !row.state.is_active() || state.version < execution.version {
            return Ok(None);
        }
        let ending = ending_of(&state);
        store
            .apply_executor_task_state(&row.id, kind, state.version, state.state, ending.clone())
            .await?;
        self.publish_on_board(&state, ending);
        Ok(Some(row))
    }

    /// Ends an executor's task that its target does not know or can no
    /// longer report, and tells the run that follows it. An end that changes
    /// no row, such as one of another kind's task or of a task that ended
    /// already, leaves the board as it is too.
    pub(crate) async fn end_executor_task(
        &self,
        kind: &str,
        task: &StoredTask,
        state: TaskState,
    ) -> Result<(), RuntimeStoreError> {
        let ending = TaskEnding {
            state,
            report: None,
            failure: None,
        };
        let version = task
            .executor
            .as_ref()
            .map_or(0, |execution| execution.version);
        let changed = self
            .inner
            .store
            .apply_executor_task_state(&task.id, kind, version, state, Some(ending.clone()))
            .await?;
        // The board shows what the row holds: a run that follows a task of
        // another kind must not see this end, and a run that follows a task
        // that ended already saw its end when the row took it.
        if !changed {
            return Ok(());
        }
        let ended = AssignedTaskState {
            task_id: task.id.clone(),
            conversation_id: task.conversation_id.clone(),
            version,
            state,
            started_at: None,
            activity: None,
            tool_calls: 0,
            approval: None,
            waits_for_lease: false,
            report: None,
            failure: None,
            ended_at: Some(self.inner.clock.now()),
        };
        self.publish_on_board(&ended, Some(ending));
        Ok(())
    }

    /// Shows a state of an executor's task on the board of the run that
    /// follows it, if a run does.
    fn publish_on_board(&self, state: &AssignedTaskState, ending: Option<TaskEnding>) {
        let Some(board) = self.inner.executors.board(&state.task_id) else {
            return;
        };
        let now = self.inner.clock.now();
        board.publish(&state.task_id, |child| {
            child.started_at = state
                .started_at
                .or(child.started_at)
                .or_else(|| (state.state != TaskState::Accepted).then_some(now));
            child.activity.clone_from(&state.activity);
            // A task's count only grows. The state of a task that ended may
            // come from its target's stored row after a reconnect, and that
            // row keeps no count.
            child.tool_calls = child.tool_calls.max(state.tool_calls);
            child.approval.clone_from(&state.approval);
            child.waits_for_lease = state.waits_for_lease;
            if let Some(ending) = ending {
                child.activity = None;
                child.approval = None;
                child.waits_for_lease = false;
                child.end = Some(ChildEnd {
                    ending,
                    ended_at: state.ended_at.unwrap_or(now),
                });
            }
        });
    }

    /// A task, for its executor.
    pub(crate) async fn executor_task(
        &self,
        task: &TaskId,
    ) -> Result<Option<StoredTask>, RuntimeStoreError> {
        self.inner.store.task(task).await
    }

    /// The tasks of an executor that have not ended, the oldest first.
    pub(crate) async fn active_executor_tasks(
        &self,
        kind: &str,
    ) -> Result<Vec<StoredTask>, RuntimeStoreError> {
        self.inner.store.active_executor_tasks(kind).await
    }

    /// Replaces the executor's document of one of its tasks. Returns whether
    /// the task is the executor's.
    pub(crate) async fn set_executor_document(
        &self,
        kind: &str,
        task: &TaskId,
        document: Option<String>,
    ) -> Result<bool, RuntimeStoreError> {
        self.inner
            .store
            .set_executor_document(task, kind, document)
            .await
    }

    /// The deletions of task conversations that wait for an executor.
    pub(crate) async fn executor_outbox(
        &self,
        kind: &str,
    ) -> Result<Vec<OutboxEntry>, RuntimeStoreError> {
        self.inner.store.executor_outbox(kind).await
    }

    /// Forgets deletions that an executor took.
    pub(crate) async fn drop_executor_outbox(
        &self,
        kind: &str,
        entries: &[OutboxEntry],
    ) -> Result<(), RuntimeStoreError> {
        self.inner.store.drop_executor_outbox(kind, entries).await
    }

    /// Forgets the deletions that wait for one target of an executor, such
    /// as a worker that the owner removed (ADR-0031, decision 9).
    pub(crate) async fn drop_executor_target_outbox(
        &self,
        kind: &str,
        target: &str,
    ) -> Result<usize, RuntimeStoreError> {
        self.inner
            .store
            .drop_executor_target_outbox(kind, target)
            .await
    }

    /// Changes whenever a deletion joins the outbox.
    pub(crate) fn outbox_changes(&self) -> watch::Receiver<u64> {
        self.inner.executors.outbox.subscribe()
    }

    /// The parent, number and executor of a task conversation; `None` for
    /// every other conversation, and when the store cannot read it.
    async fn task_origin(&self, conversation: &ConversationId) -> Option<TaskConversation> {
        match self.inner.store.conversation_summary(conversation).await {
            Ok(summary) => summary.and_then(|summary| summary.task),
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not read a task conversation");
                None
            }
        }
    }

    /// Whether a conversation is in full access; `false` when the store
    /// cannot read it.
    async fn in_full_access(&self, conversation: &ConversationId) -> bool {
        match self.inner.store.conversation_summary(conversation).await {
            Ok(summary) => summary.is_some_and(|summary| summary.full_access),
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not read whether a conversation is in full access");
                false
            }
        }
    }

    /// Lends the credential of a provider profile for a task, from the vault
    /// (ADR-0010). `None` for a profile without a credential.
    async fn lend_credential(
        &self,
        profile_id: &ProfileId,
    ) -> Result<Option<credentials::LentCredential>, credentials::LendError> {
        let profile = match self.inner.store.profile(profile_id).await {
            Ok(Some(profile)) => profile,
            Ok(None) => return Err(credentials::LendError::ProfileMissing),
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not read the profile of a credential to lend");
                return Err(credentials::LendError::StorageFailed);
            }
        };
        let Some(reference) = profile.credential_ref.clone() else {
            return Ok(None);
        };
        let credential = in_vault(&self.inner.vault, move |vault| vault.read(&reference))
            .await
            .map_err(|error| match error {
                VaultError::Missing | VaultError::Unavailable(_) => {
                    credentials::LendError::CredentialUnavailable
                }
            })?;
        Ok(Some(credentials::LentCredential::new(
            profile.binding(),
            credential,
        )))
    }

    /// Lends the plugin of `kind` the credential of `profile` for `task`
    /// (ADR-0010; ADR-0034, decision 5), only while the task is an active
    /// task of the plugin's executor and the run that gave it uses the
    /// profile. The store's rows decide, which the plugin cannot change: its
    /// own document of the task does not, and neither does a profile that
    /// the parent conversation chose after the run.
    async fn lend_for_task(
        &self,
        kind: &str,
        task: &TaskId,
        profile: &ProfileId,
    ) -> Result<Option<credentials::LentCredential>, credentials::LendError> {
        let recorded = match self.inner.store.executor_task_profile(task, kind).await {
            Ok(recorded) => recorded,
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), plugin = kind, task = %task, "could not read the task of a credential to lend");
                return Err(credentials::LendError::StorageFailed);
            }
        };
        let refusal = match recorded {
            None => Some("not_the_plugins_task"),
            Some(recorded) if !recorded.active => Some("task_ended"),
            Some(recorded) if recorded.profile_id.as_ref() != Some(profile) => {
                Some("another_profile")
            }
            Some(_recorded) => None,
        };
        if let Some(reason) = refusal {
            tracing::warn!(plugin = kind, task = %task, profile = %profile, reason, "refused to lend a credential for a task");
            return Err(credentials::LendError::NotPermitted);
        }
        let Some(lent) = self.lend_credential(profile).await? else {
            tracing::info!(plugin = kind, task = %task, profile = %profile, "the profile of a task has no credential to lend");
            return Ok(None);
        };
        tracing::info!(plugin = kind, task = %task, profile = %profile, "lent a credential for a task");
        Ok(Some(lent))
    }

    /// The executor host bound to the executor of `kind`, as the service
    /// `executor-host` binds it to the plugin with that ID (ADR-0034,
    /// decision 5).
    pub(crate) fn bound_executor_host(&self, kind: &str) -> Arc<dyn ExecutorHostApi> {
        Arc::new(BoundExecutorHost {
            runtime: self.clone(),
            kind: kind.to_owned(),
        })
    }

    /// The credentials bound to the executor of `kind`, as the service
    /// `credentials` binds them to the plugin with that ID (ADR-0034,
    /// decision 5).
    pub(crate) fn bound_credentials(&self, kind: &str) -> Arc<dyn CredentialsApi> {
        Arc::new(BoundCredentials {
            runtime: self.clone(),
            kind: kind.to_owned(),
        })
    }
}

/// The code of a call that the executor host refuses because the task is not
/// a task of the plugin's executor.
const NOT_PERMITTED: &str = "not_permitted";

/// The runtime's provider of the service `executor-host` (ADR-0034,
/// decision 5).
#[derive(Debug)]
pub(crate) struct ExecutorHostProvider {
    runtime: Runtime,
}

impl ExecutorHostProvider {
    /// The provider of `runtime`'s executor host.
    pub(crate) fn new(runtime: &Runtime) -> Self {
        Self {
            runtime: runtime.clone(),
        }
    }
}

impl ServiceProvider for ExecutorHostProvider {
    fn name(&self) -> &'static str {
        ExecutorHost::NAME
    }

    fn major(&self) -> u16 {
        ExecutorHost::MAJOR
    }

    /// Binds the executor host to the executor kind that equals the plugin's
    /// ID, which the SDK's rule `PluginKind` gives the plugin's executor.
    fn bind(&self, binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        Ok(BoundService::new::<ExecutorHost>(
            self.runtime.bound_executor_host(binding.id.as_str()),
        ))
    }
}

/// The runtime's provider of the service `credentials` (ADR-0034,
/// decision 5; ADR-0010).
#[derive(Debug)]
pub(crate) struct CredentialsProvider {
    runtime: Runtime,
}

impl CredentialsProvider {
    /// The provider of `runtime`'s credentials.
    pub(crate) fn new(runtime: &Runtime) -> Self {
        Self {
            runtime: runtime.clone(),
        }
    }
}

impl ServiceProvider for CredentialsProvider {
    fn name(&self) -> &'static str {
        Credentials::NAME
    }

    fn major(&self) -> u16 {
        Credentials::MAJOR
    }

    /// Binds the credentials to the executor kind that equals the plugin's
    /// ID, so the plugin borrows only for its own executor's tasks.
    fn bind(&self, binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        Ok(BoundService::new::<Credentials>(
            self.runtime.bound_credentials(binding.id.as_str()),
        ))
    }
}

/// The executor host of one executor kind: every call answers only for the
/// tasks, the documents and the outbox of that kind.
#[derive(Debug)]
struct BoundExecutorHost {
    runtime: Runtime,
    kind: String,
}

/// A store failure of the executor host, as the code that the plugin logs.
/// The runtime logs the failure itself, because the plugin never sees more
/// than the code.
fn host_failure(kind: &str, error: &RuntimeStoreError) -> HostFailure {
    tracing::error!(%error, error.code = error.code(), plugin = kind, "the executor host could not reach the store");
    HostFailure::new(error.code())
}

/// A task's row as the executor of `kind` sees it; `None` for a task of
/// another kind or of no executor.
fn executor_task_of(row: StoredTask, kind: &str) -> Option<ExecutorTask> {
    let execution = row.executor?;
    (execution.kind == kind).then(|| {
        ExecutorTask::new(
            row.id,
            row.conversation_id,
            row.state,
            execution.target,
            execution.document,
        )
    })
}

impl ExecutorHostApi for BoundExecutorHost {
    fn publish_task_state(
        &self,
        state: AssignedTaskState,
    ) -> BoxFuture<'static, Result<Option<ExecutorTask>, HostFailure>> {
        let runtime = self.runtime.clone();
        let kind = self.kind.clone();
        async move {
            let before = runtime
                .publish_task_state(&kind, state)
                .await
                .map_err(|error| host_failure(&kind, &error))?;
            Ok(before.and_then(|row| executor_task_of(row, &kind)))
        }
        .boxed()
    }

    fn end_task(
        &self,
        task: TaskId,
        state: TaskState,
    ) -> BoxFuture<'static, Result<(), HostFailure>> {
        let runtime = self.runtime.clone();
        let kind = self.kind.clone();
        async move {
            let row = runtime
                .executor_task(&task)
                .await
                .map_err(|error| host_failure(&kind, &error))?;
            let own = row.filter(|row| {
                row.executor
                    .as_ref()
                    .is_some_and(|execution| execution.kind == kind)
            });
            let Some(row) = own else {
                tracing::warn!(plugin = %kind, task = %task, "refused to end a task that is not a task of the plugin's executor");
                return Err(HostFailure::new(NOT_PERMITTED));
            };
            runtime
                .end_executor_task(&kind, &row, state)
                .await
                .map_err(|error| host_failure(&kind, &error))
        }
        .boxed()
    }

    fn task(&self, task: TaskId) -> BoxFuture<'static, Result<Option<ExecutorTask>, HostFailure>> {
        let runtime = self.runtime.clone();
        let kind = self.kind.clone();
        async move {
            let row = runtime
                .executor_task(&task)
                .await
                .map_err(|error| host_failure(&kind, &error))?;
            Ok(row.and_then(|row| executor_task_of(row, &kind)))
        }
        .boxed()
    }

    fn active_tasks(&self) -> BoxFuture<'static, Result<Vec<ExecutorTask>, HostFailure>> {
        let runtime = self.runtime.clone();
        let kind = self.kind.clone();
        async move {
            let rows = runtime
                .active_executor_tasks(&kind)
                .await
                .map_err(|error| host_failure(&kind, &error))?;
            Ok(rows
                .into_iter()
                .filter_map(|row| executor_task_of(row, &kind))
                .collect())
        }
        .boxed()
    }

    fn set_document(
        &self,
        task: TaskId,
        document: Option<String>,
    ) -> BoxFuture<'static, Result<bool, HostFailure>> {
        let runtime = self.runtime.clone();
        let kind = self.kind.clone();
        async move {
            runtime
                .set_executor_document(&kind, &task, document)
                .await
                .map_err(|error| host_failure(&kind, &error))
        }
        .boxed()
    }

    fn outbox(&self) -> BoxFuture<'static, Result<Vec<OutboxItem>, HostFailure>> {
        let runtime = self.runtime.clone();
        let kind = self.kind.clone();
        async move {
            let entries = runtime
                .executor_outbox(&kind)
                .await
                .map_err(|error| host_failure(&kind, &error))?;
            Ok(entries
                .into_iter()
                .map(|entry| OutboxItem::new(entry.target, entry.conversation_id))
                .collect())
        }
        .boxed()
    }

    fn drop_outbox(&self, items: Vec<OutboxItem>) -> BoxFuture<'static, Result<(), HostFailure>> {
        let runtime = self.runtime.clone();
        let kind = self.kind.clone();
        async move {
            let entries: Vec<OutboxEntry> = items
                .into_iter()
                .map(|item| OutboxEntry {
                    target: item.target,
                    conversation_id: item.conversation_id,
                })
                .collect();
            runtime
                .drop_executor_outbox(&kind, &entries)
                .await
                .map_err(|error| host_failure(&kind, &error))
        }
        .boxed()
    }

    fn drop_target_outbox(&self, target: String) -> BoxFuture<'static, Result<u64, HostFailure>> {
        let runtime = self.runtime.clone();
        let kind = self.kind.clone();
        async move {
            let dropped = runtime
                .drop_executor_target_outbox(&kind, &target)
                .await
                .map_err(|error| host_failure(&kind, &error))?;
            Ok(u64::try_from(dropped).unwrap_or(u64::MAX))
        }
        .boxed()
    }

    fn outbox_changes(&self) -> watch::Receiver<u64> {
        self.runtime.outbox_changes()
    }

    fn parent_has_full_access(&self, conversation: ConversationId) -> BoxFuture<'static, bool> {
        let runtime = self.runtime.clone();
        let kind = self.kind.clone();
        async move {
            let Some(origin) = runtime.task_origin(&conversation).await else {
                return false;
            };
            // Only a task conversation that this kind's executor serves
            // answers, so a plugin learns nothing of other conversations.
            let served = origin
                .executor
                .as_ref()
                .is_some_and(|executor| executor.kind == kind);
            served && runtime.in_full_access(&origin.parent_id).await
        }
        .boxed()
    }
}

/// The credentials of one executor kind: a lend only for an active task of
/// that kind whose run uses the profile.
#[derive(Debug)]
struct BoundCredentials {
    runtime: Runtime,
    kind: String,
}

impl CredentialsApi for BoundCredentials {
    fn lend(
        &self,
        task: TaskId,
        profile: ProfileId,
    ) -> BoxFuture<'static, Result<Option<credentials::LentCredential>, credentials::LendError>>
    {
        let runtime = self.runtime.clone();
        let kind = self.kind.clone();
        async move { runtime.lend_for_task(&kind, &task, &profile).await }.boxed()
    }
}
