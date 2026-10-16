//! Task executors (ADR-0031, decision 4): what a plugin registers to run the
//! tasks of a reply somewhere else, such as on a worker, and the runtime's
//! side of following those tasks.
//!
//! Child agents are the runtime's own executor. An executor's task is a task
//! of ADR-0024 all the same: a `task` part with a number among the
//! conversation's children, on the run's board and in its lane, with
//! background tasks, follow-ups and waits. The executor submits, follows and
//! cancels its tasks, keeps what it needs in a document on the task's row,
//! and publishes each state through [`Runtime::publish_task_state`]; the
//! runtime stores the state in the task's row and shows it on the board of
//! the run that follows the task.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use mannie_provider::ToolDefinition;
use mannie_store::{OutboxEntry, RuntimeStoreError, StoredTask, TaskEnding};
use mannie_types::chat::{ChatError, CheckpointRef, EarlierTurns, LoadEarlierTurns};
use mannie_types::ids::{ConversationId, ProfileId, TaskId};
use mannie_types::secret::Secret;
use mannie_types::tasks::{AssignedTaskState, ProfileSnapshot, TaskExecutorRef, TaskState};
use mannie_types::tools::{DecideApproval, ReadToolOutput, ToolOutputPage, ToolsError};
use mannie_types::transcript::{Checkpoint, ConversationSummary, ConversationUpdate};
use tokio::sync::watch;

use crate::Runtime;
use crate::profiles::{VaultError, in_vault};
use crate::tasks::{ChildEnd, TaskBoard};

/// A call of an executor's tool, as the executor plans it.
#[derive(Debug, Clone, Copy)]
pub struct PlanCall<'a> {
    /// The call's arguments as the model wrote them, without the activity.
    pub arguments: &'a str,
    /// The provider profile of the reply that makes the call, without its
    /// credential.
    pub profile: &'a ProfileSnapshot,
}

/// A task that an executor takes on, as it planned a call of its tool.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskPlan {
    /// The target that runs the task, such as a worker.
    pub target: TaskExecutorRef,
    /// The task's title, cleaned for the owner.
    pub title: String,
    /// The instruction.
    pub instruction: String,
    /// Whether the reply continues while the target works.
    pub background: bool,
    /// What else the executor needs to submit the task, which the runtime
    /// passes on as it is.
    pub options: serde_json::Value,
}

/// The target of a follow-up, as an executor takes it on.
#[derive(Debug, Clone, PartialEq)]
pub struct FollowUpPlan {
    /// The target that runs the earlier task, as it is named now.
    pub target: TaskExecutorRef,
    /// What else the executor needs to submit the task.
    pub options: serde_json::Value,
}

/// Why an executor did not plan a call of its tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
    /// The arguments are not valid. The call does not run, as a call of any
    /// tool with arguments that are not valid does.
    InvalidArguments {
        /// What is wrong, for the model.
        detail: String,
    },
    /// The executor refused the task.
    Refused(ExecutorRefusal),
}

/// Why a submission did not return a first state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitError {
    /// The executor refused the task, which its target never stored.
    Refused(ExecutorRefusal),
    /// The executor's plugin stopped before the executor knew whether the
    /// target stored the task. The run lets the task go, as quitting does,
    /// and the executor learns what became of it when it starts again.
    Stopped,
}

/// Why an executor did not take a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorRefusal {
    /// The executor's code for the refusal, which the UI shows a message of
    /// the plugin's for.
    pub code: String,
    /// The name of the target that the call named, if it named one.
    pub target: Option<String>,
    /// The result that the model reads.
    pub text: String,
}

/// A task that the runtime stored for an executor, which the executor
/// submits to its target.
#[derive(Debug, Clone)]
pub struct TaskSubmission {
    /// The task, which the runtime chose and stored first, so a submission
    /// that is sent again finds the same task.
    pub task_id: TaskId,
    /// The task's conversation.
    pub conversation_id: ConversationId,
    /// The parent conversation.
    pub parent_id: ConversationId,
    /// The task's number among the parent's children.
    pub number: u32,
    /// Whether the task continues an earlier task of its conversation.
    pub follow_up: bool,
    /// The task's title.
    pub title: String,
    /// The instruction.
    pub instruction: String,
    /// The target.
    pub target: TaskExecutorRef,
    /// What the executor's plan added.
    pub options: serde_json::Value,
    /// The provider profile of the reply that gave the task, without its
    /// credential.
    pub profile: ProfileSnapshot,
}

/// The instruction section that an executor adds for the runs that are
/// offered its tool, as it is when a run starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorSection {
    /// Where the section goes among the sections.
    pub position: u32,
    /// The section's text.
    pub text: String,
}

/// What a plugin registers to run tasks somewhere else (ADR-0031,
/// decision 4). Every call may fail with the target unreachable; the calls
/// that read a task conversation answer with that conversation's errors.
pub trait TaskExecutor: Send + Sync + fmt::Debug + 'static {
    /// The executor's kind: the ID of the plugin that registered it.
    fn kind(&self) -> &'static str;

    /// The agent that the runtime records for the executor's tasks.
    fn agent(&self) -> &'static str;

    /// The executor's tool, as a run that starts now is offered it, or
    /// `None` while the executor offers none. The runtime reads it once for
    /// each run.
    fn tool(&self) -> Option<ToolDefinition>;

    /// The instruction section for the runs that are offered the tool, read
    /// once for each run, or `None`.
    fn section(&self) -> Option<ExecutorSection>;

    /// Whether the executor's tasks outlive the reply that gave them, as a
    /// worker's do (ADR-0027, decision 6): a stop cancels them without
    /// waiting longer than the runtime's cancel timeout, and quitting leaves
    /// them running.
    fn outlives_reply(&self) -> bool;

    /// Takes on a call of the executor's tool, or refuses it. The runtime
    /// checks the reply's task budget between arguments that are not valid
    /// and a refusal, as it does for `start_task`.
    fn plan(&self, call: PlanCall<'_>) -> Result<TaskPlan, PlanError>;

    /// Takes on a follow-up of an earlier task of the executor's, on the
    /// target that ran it, or refuses it.
    fn plan_follow_up(
        &self,
        earlier: &TaskExecutorRef,
        profile: &ProfileSnapshot,
    ) -> Result<FollowUpPlan, ExecutorRefusal>;

    /// The executor's document of a new task, which the runtime stores with
    /// the task before it submits it.
    fn document(&self, submission: &TaskSubmission) -> Option<String>;

    /// Submits a task that the runtime stored, which the target stores once
    /// for its ID, and returns its first state. A target that cannot be
    /// reached is tried again until the executor gives up; a stop of the run
    /// drops the future.
    fn submit(
        &self,
        submission: TaskSubmission,
    ) -> BoxFuture<'static, Result<AssignedTaskState, SubmitError>>;

    /// Asks the target to cancel a task. The executor keeps the cancel until
    /// the target takes it or the task ends.
    fn cancel(&self, task: &TaskId) -> BoxFuture<'static, ()>;

    /// Follows a task conversation on its target, from a cursor or from a
    /// snapshot. The stream ends when the target can no longer be reached.
    fn subscribe(
        &self,
        target: &TaskExecutorRef,
        conversation: &ConversationId,
        after_seq: Option<u32>,
    ) -> BoxFuture<'static, Result<BoxStream<'static, ConversationUpdate>, ChatError>>;

    /// Earlier turns of a task conversation on its target.
    fn earlier_turns(
        &self,
        target: &TaskExecutorRef,
        input: LoadEarlierTurns,
    ) -> BoxFuture<'static, Result<EarlierTurns, ChatError>>;

    /// A checkpoint of a task conversation on its target.
    fn checkpoint(
        &self,
        target: &TaskExecutorRef,
        input: CheckpointRef,
    ) -> BoxFuture<'static, Result<Checkpoint, ChatError>>;

    /// A page of an archived output of a task conversation on its target.
    fn tool_output(
        &self,
        target: &TaskExecutorRef,
        input: ReadToolOutput,
    ) -> BoxFuture<'static, Result<ToolOutputPage, ToolsError>>;

    /// The owner's decision about a call of a task on its target.
    fn decide_approval(
        &self,
        target: &TaskExecutorRef,
        input: DecideApproval,
    ) -> BoxFuture<'static, Result<(), ToolsError>>;
}

/// An executor whose kind is registered already.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("an executor of the kind {kind} is already registered")]
pub struct DuplicateExecutor {
    /// The kind.
    pub kind: &'static str,
}

/// A provider credential that the runtime lends an executor for one task,
/// with the binding of the profile that it belongs to.
#[derive(Debug)]
pub struct LentCredential {
    /// The profile and its configuration revision.
    pub binding: String,
    /// The credential.
    pub credential: Secret,
}

/// Why the runtime could not lend a credential.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LendError {
    /// The profile no longer exists.
    #[error("the profile no longer exists")]
    ProfileMissing,
    /// The credential could not be read.
    #[error("the credential could not be read")]
    CredentialUnavailable,
    /// The store failed.
    #[error("the store failed")]
    StorageFailed,
}

/// The board of a run that follows an executor's task.
#[derive(Debug)]
struct FollowingBoard {
    /// The kind of the executor that runs the task.
    kind: String,
    board: Arc<TaskBoard>,
}

/// The registered executors, and the boards of the runs that follow their
/// tasks.
#[derive(Debug)]
pub(crate) struct Executors {
    registered: RwLock<Vec<Arc<dyn TaskExecutor>>>,
    /// The boards of the runs that follow executors' tasks, by task.
    boards: Mutex<HashMap<TaskId, FollowingBoard>>,
    /// Counts the deletions that wait in the outbox, so each executor takes
    /// them while its plugin is on.
    outbox: watch::Sender<u64>,
}

impl Default for Executors {
    fn default() -> Self {
        Self {
            registered: RwLock::default(),
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

impl Executors {
    fn board(&self, task: &TaskId) -> Option<Arc<TaskBoard>> {
        lock(&self.boards)
            .get(task)
            .map(|following| Arc::clone(&following.board))
    }

    /// Tells each run that follows a task of `kind` to let the task go,
    /// because the executor's plugin stopped.
    fn let_go_of(&self, kind: &str) -> usize {
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

impl Runtime {
    /// Registers an executor, whose tool the runs that start from now on are
    /// offered (ADR-0030, decision 5). Kinds are unique.
    pub fn add_executor(&self, executor: Arc<dyn TaskExecutor>) -> Result<(), DuplicateExecutor> {
        let mut registered = self
            .inner
            .executors
            .registered
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        if registered
            .iter()
            .any(|existing| existing.kind() == executor.kind())
        {
            return Err(DuplicateExecutor {
                kind: executor.kind(),
            });
        }
        tracing::info!(
            kind = executor.kind(),
            "an executor of tasks was registered"
        );
        registered.push(executor);
        Ok(())
    }

    /// Removes the executor of a kind, as its plugin stops (ADR-0031,
    /// decision 10). Runs that started while it was registered keep its
    /// tool, and their calls of it are refused because its plugin is off.
    /// Each run that follows one of its tasks lets the task go, as quitting
    /// does: the task may still run on its target, and a wait takes it up
    /// again once the plugin is back on.
    pub fn remove_executor(&self, kind: &str) -> Option<Arc<dyn TaskExecutor>> {
        let removed = {
            let mut registered = self
                .inner
                .executors
                .registered
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            let index = registered
                .iter()
                .position(|executor| executor.kind() == kind)?;
            registered.remove(index)
        };
        // After the removal, so a run that starts following a task from now
        // on finds the executor gone and lets the task go itself.
        let let_go = self.inner.executors.let_go_of(kind);
        tracing::info!(
            kind,
            tasks_let_go = let_go,
            "an executor of tasks was removed"
        );
        Some(removed)
    }

    /// The executor of a kind, while it is registered.
    pub(crate) fn executor(&self, kind: &str) -> Option<Arc<dyn TaskExecutor>> {
        self.inner
            .executors
            .registered
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|executor| executor.kind() == kind)
            .map(Arc::clone)
    }

    /// The executors as they are registered now.
    pub(crate) fn executors_now(&self) -> Vec<Arc<dyn TaskExecutor>> {
        self.inner
            .executors
            .registered
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
    /// longer report, and tells the run that follows it.
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
        self.inner
            .store
            .apply_executor_task_state(&task.id, kind, version, state, Some(ending.clone()))
            .await?;
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

    /// Whether the parent of a task conversation is in full access
    /// (ADR-0029, decision 5).
    pub(crate) async fn parent_has_full_access(&self, conversation: &ConversationId) -> bool {
        let store = &self.inner.store;
        let parent = match store.conversation_summary(conversation).await {
            Ok(Some(summary)) => summary.task.map(|task| task.parent_id),
            Ok(None) => None,
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not read a task conversation");
                None
            }
        };
        let Some(parent) = parent else {
            return false;
        };
        match store.conversation_summary(&parent).await {
            Ok(summary) => summary.is_some_and(|summary| summary.full_access),
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not read whether a conversation is in full access");
                false
            }
        }
    }

    /// Lends the credential of a provider profile for a task, from the vault
    /// (ADR-0010). `None` for a profile without a credential.
    pub(crate) async fn lend_credential(
        &self,
        profile_id: &ProfileId,
    ) -> Result<Option<LentCredential>, LendError> {
        let profile = match self.inner.store.profile(profile_id).await {
            Ok(Some(profile)) => profile,
            Ok(None) => return Err(LendError::ProfileMissing),
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not read the profile of a credential to lend");
                return Err(LendError::StorageFailed);
            }
        };
        let Some(reference) = profile.credential_ref.clone() else {
            return Ok(None);
        };
        let credential = in_vault(&self.inner.vault, move |vault| vault.read(&reference))
            .await
            .map_err(|error| match error {
                VaultError::Missing | VaultError::Unavailable(_) => {
                    LendError::CredentialUnavailable
                }
            })?;
        Ok(Some(LentCredential {
            binding: profile.binding(),
            credential,
        }))
    }

    /// The services of the runtime for a plugin's executor, which the
    /// composition root grants to the plugins that register one (ADR-0030,
    /// decision 13).
    pub fn executor_services(&self) -> ExecutorServices {
        ExecutorServices {
            runtime: self.clone(),
        }
    }

    /// Lends provider credentials for the tasks of a plugin's executor. The
    /// composition root grants one only to the plugins that it trusts with
    /// provider credentials (ADR-0010; ADR-0030, decision 13).
    pub fn credential_lender(&self) -> CredentialLender {
        CredentialLender {
            runtime: self.clone(),
        }
    }
}

/// What a plugin's executor uses of the runtime (ADR-0031, decision 4): the
/// rows and states of its tasks, the outbox of deletions, and whether a
/// task's parent conversation is in full access. A plugin receives these and
/// never the runtime itself (ADR-0030, decision 13).
#[derive(Debug, Clone)]
pub struct ExecutorServices {
    runtime: Runtime,
}

impl ExecutorServices {
    /// Publishes a state of one of the executor's tasks: the runtime stores
    /// it in the task's row and shows it on the board of the run that follows
    /// the task. A state of a task that is another executor's or another
    /// conversation's, of a task that ended, or older than the one stored
    /// changes nothing.
    ///
    /// Returns the task's row as it was before the state, for a state that
    /// the runtime took.
    pub async fn publish_task_state(
        &self,
        kind: &str,
        state: AssignedTaskState,
    ) -> Result<Option<StoredTask>, RuntimeStoreError> {
        self.runtime.publish_task_state(kind, state).await
    }

    /// Ends a task that its target does not know or can no longer report,
    /// and tells the run that follows it.
    pub async fn end_executor_task(
        &self,
        kind: &str,
        task: &StoredTask,
        state: TaskState,
    ) -> Result<(), RuntimeStoreError> {
        self.runtime.end_executor_task(kind, task, state).await
    }

    /// A task, by its ID.
    pub async fn executor_task(
        &self,
        task: &TaskId,
    ) -> Result<Option<StoredTask>, RuntimeStoreError> {
        self.runtime.executor_task(task).await
    }

    /// The tasks of an executor that have not ended, the oldest first.
    pub async fn active_executor_tasks(
        &self,
        kind: &str,
    ) -> Result<Vec<StoredTask>, RuntimeStoreError> {
        self.runtime.active_executor_tasks(kind).await
    }

    /// Replaces the executor's document of one of its tasks. Returns whether
    /// the task is the executor's.
    pub async fn set_executor_document(
        &self,
        kind: &str,
        task: &TaskId,
        document: Option<String>,
    ) -> Result<bool, RuntimeStoreError> {
        self.runtime
            .set_executor_document(kind, task, document)
            .await
    }

    /// The deletions of task conversations that wait for an executor.
    pub async fn executor_outbox(&self, kind: &str) -> Result<Vec<OutboxEntry>, RuntimeStoreError> {
        self.runtime.executor_outbox(kind).await
    }

    /// Forgets deletions that an executor took.
    pub async fn drop_executor_outbox(
        &self,
        kind: &str,
        entries: &[OutboxEntry],
    ) -> Result<(), RuntimeStoreError> {
        self.runtime.drop_executor_outbox(kind, entries).await
    }

    /// Forgets the deletions that wait for one target of an executor, such
    /// as a worker that the owner removed (ADR-0031, decision 9).
    pub async fn drop_executor_target_outbox(
        &self,
        kind: &str,
        target: &str,
    ) -> Result<usize, RuntimeStoreError> {
        self.runtime.drop_executor_target_outbox(kind, target).await
    }

    /// Changes whenever a deletion joins the outbox.
    pub fn outbox_changes(&self) -> watch::Receiver<u64> {
        self.runtime.outbox_changes()
    }

    /// Whether the parent of a task conversation is in full access
    /// (ADR-0029, decision 5).
    pub async fn parent_has_full_access(&self, conversation: &ConversationId) -> bool {
        self.runtime.parent_has_full_access(conversation).await
    }
}

/// Lends a plugin's executor the credential of a provider profile for one of
/// its tasks, as a worker's lease (ADR-0010; ADR-0030, decision 13).
#[derive(Debug, Clone)]
pub struct CredentialLender {
    runtime: Runtime,
}

impl CredentialLender {
    /// The credential of a profile with its binding, or `None` for a
    /// profile without a credential.
    pub async fn lend_credential(
        &self,
        profile_id: &ProfileId,
    ) -> Result<Option<LentCredential>, LendError> {
        self.runtime.lend_credential(profile_id).await
    }
}
