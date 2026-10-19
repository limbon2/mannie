//! Tasks that the master assigns to this worker (ADR-0028, decision 1;
//! plugins/workers/docs/delegation.md): assigning one by the IDs that the
//! master chose, its state as the master follows it, cancelling it, its
//! credential lease, deleting its conversation, and letting every task go
//! when the worker is unpaired.
//!
//! An assigned task runs in a conversation of its own on the worker, as a
//! child's does, under the worker agent's definition and the host's ceiling.
//! Its run publishes the task's state here, and the master follows the states
//! of every assigned task through one watch.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mannie_provider::{check_base_url, credential_transport_allowed};
use mannie_store::{
    Change, Durability, NewConversation, NewRun, NewTask, RuntimeStoreError, StoredTask,
};
use mannie_tools::clean_label;
use mannie_types::ids::{ConversationId, TaskId, TurnId};
use mannie_types::secret::Secret;
use mannie_types::tasks::{AssignedTaskState, MAX_TITLE_CHARS};
use mannie_types::tasks::{ProfileSnapshot, TaskAssignment};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};

use crate::actor::{ChildLink, Command};
use crate::profiles::{VaultError, in_vault};
use crate::tasks::{ChildGuard, ChildState, TaskSink, check_instruction};
use crate::{Busy, Runtime, lock};

/// What a worker keeps of an assignment in its task's row: what the task's
/// runs need besides the instruction, which is the first message of its turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AssignmentRecord {
    profile: ProfileSnapshot,
    read_only: bool,
}

/// What the run of an assigned task keeps of its assignment.
#[derive(Debug, Clone)]
pub(crate) struct AssignedRun {
    /// The task.
    pub(crate) task: TaskId,
    /// The profile that the master sent with the task.
    pub(crate) profile: ProfileSnapshot,
    /// Whether the task may only read and search.
    pub(crate) read_only: bool,
}

/// The reference under which a task's credential lease is kept in the vault
/// (plugins/workers/docs/credential-leases.md, the lease).
pub(crate) fn lease_reference(task: &TaskId) -> String {
    format!("lease/{task}")
}

/// Why the worker did not take a task.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AssignError {
    /// The runtime is not a worker's.
    #[error("this runtime does not run assigned tasks")]
    NotAWorker,
    /// The task's ID or its conversation's ID belongs to another task.
    #[error("the task's IDs belong to another task")]
    Conflict,
    /// A follow-up names a conversation that the worker does not have.
    #[error("the conversation of the follow-up does not exist")]
    ConversationNotFound,
    /// The conversation still works on a task.
    #[error("the conversation still works on a task")]
    StillRunning,
    /// The assignment is not valid.
    #[error("the assignment is not valid: {detail}")]
    Invalid {
        /// What is wrong.
        detail: String,
    },
    /// The worker is quitting.
    #[error("the worker is quitting")]
    ShuttingDown,
    /// The worker's store failed. The log has the cause.
    #[error("the worker's store failed")]
    StorageFailed,
}

impl AssignError {
    /// The stable code of the error, for logs and the link.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotAWorker => "assign.not_a_worker",
            Self::Conflict => "assign.conflict",
            Self::ConversationNotFound => "assign.conversation_not_found",
            Self::StillRunning => "assign.still_running",
            Self::Invalid { .. } => "assign.invalid",
            Self::ShuttingDown => "assign.shutting_down",
            Self::StorageFailed => "assign.storage_failed",
        }
    }

    fn invalid(detail: impl Into<String>) -> Self {
        Self::Invalid {
            detail: detail.into(),
        }
    }
}

/// Why a lease was not taken or dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LeaseError {
    /// The lease names a task that is not an active assigned task.
    #[error("the task is not an active task of this worker")]
    TaskNotActive,
    /// The lease's binding is not the binding of the task's profile.
    #[error("the lease's binding is not the task's")]
    BindingMismatch,
    /// The lease vault cannot be used.
    #[error("the lease vault cannot be used")]
    VaultUnavailable,
    /// The worker's store failed. The log has the cause.
    #[error("the worker's store failed")]
    StorageFailed,
}

impl LeaseError {
    /// The stable code of the error, for logs and the link.
    pub fn code(self) -> &'static str {
        match self {
            Self::TaskNotActive => "lease.task_not_active",
            Self::BindingMismatch => "lease.binding_mismatch",
            Self::VaultUnavailable => "lease.vault_unavailable",
            Self::StorageFailed => "lease.storage_failed",
        }
    }
}

fn storage_failure(error: &RuntimeStoreError) -> AssignError {
    tracing::error!(%error, error.code = error.code(), "an assigned task could not be read or saved");
    AssignError::StorageFailed
}

fn lease_storage_failure(error: &RuntimeStoreError) -> LeaseError {
    tracing::error!(%error, error.code = error.code(), "the task of a lease could not be read");
    LeaseError::StorageFailed
}

/// The live state of an assigned task while it is active.
#[derive(Debug, Clone)]
struct LiveTask {
    conversation_id: ConversationId,
    /// The version of the task's latest stored state.
    version: u64,
    state: ChildState,
}

#[derive(Debug, Default)]
struct Shared {
    live: HashMap<TaskId, LiveTask>,
    watchers: Vec<mpsc::Sender<AssignedTaskState>>,
}

/// The assigned tasks of a worker: the live state of each active task, and
/// the watchers that the master follows them through.
#[derive(Debug, Default)]
pub(crate) struct AssignedTasks {
    shared: Mutex<Shared>,
}

fn view(task: &TaskId, live: &LiveTask) -> AssignedTaskState {
    let end = live.state.end.as_ref();
    AssignedTaskState {
        task_id: task.clone(),
        conversation_id: live.conversation_id.clone(),
        version: live.version,
        state: live.state.task_state(),
        started_at: live.state.started_at,
        activity: live.state.activity.clone(),
        tool_calls: live.state.tool_calls,
        approval: live.state.approval.clone(),
        waits_for_lease: live.state.waits_for_lease,
        report: end.and_then(|end| end.ending.report.clone()),
        failure: end.and_then(|end| end.ending.failure.clone()),
        ended_at: end.map(|end| end.ended_at),
    }
}

/// Sends a state to every watcher, and drops the watchers whose queue is
/// full or who went away. A dropped watcher sees its watch end and watches
/// again.
fn broadcast(watchers: &mut Vec<mpsc::Sender<AssignedTaskState>>, state: &AssignedTaskState) {
    watchers.retain(|watcher| watcher.try_send(state.clone()).is_ok());
}

impl AssignedTasks {
    fn shared(&self) -> MutexGuard<'_, Shared> {
        // Every critical section only replaces entries, so the map stays
        // consistent even if a holder panicked.
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A task that was stored, before its run starts.
    pub(crate) fn accepted(&self, task: &TaskId, conversation: &ConversationId, version: u64) {
        let live = LiveTask {
            conversation_id: conversation.clone(),
            version,
            state: ChildState::default(),
        };
        let mut shared = self.shared();
        let state = view(task, &live);
        shared.live.insert(task.clone(), live);
        broadcast(&mut shared.watchers, &state);
    }

    /// Records the version of a task's latest stored state. The states that
    /// follow carry it.
    pub(crate) fn set_version(&self, task: &TaskId, version: u64) {
        if let Some(live) = self.shared().live.get_mut(task) {
            live.version = live.version.max(version);
        }
    }

    /// Changes an active task's state and tells the watchers. A task that
    /// ended leaves the live states: its stored row has its end.
    pub(crate) fn publish(&self, task: &TaskId, change: impl FnOnce(&mut ChildState)) {
        let mut shared = self.shared();
        let Some(live) = shared.live.get_mut(task) else {
            return;
        };
        change(&mut live.state);
        let state = view(task, live);
        if live.state.end.is_some() {
            shared.live.remove(task);
        }
        broadcast(&mut shared.watchers, &state);
    }

    /// The live state of an active task.
    fn get(&self, task: &TaskId) -> Option<AssignedTaskState> {
        let shared = self.shared();
        shared.live.get(task).map(|live| view(task, live))
    }

    /// The conversations of the active tasks.
    fn active_conversations(&self) -> Vec<ConversationId> {
        self.shared()
            .live
            .values()
            .map(|live| live.conversation_id.clone())
            .collect()
    }

    /// A watch that starts with the states of the active tasks, in the order
    /// of their versions, and then gets every change. Both happen under one
    /// lock, so no change falls between them.
    fn watch(&self, queue: usize) -> AssignedTaskWatch {
        let (sender, receiver) = mpsc::channel(queue);
        let mut shared = self.shared();
        let mut initial: Vec<AssignedTaskState> = shared
            .live
            .iter()
            .map(|(task, live)| view(task, live))
            .collect();
        initial.sort_by(|left, right| {
            (left.version, &left.task_id).cmp(&(right.version, &right.task_id))
        });
        shared.watchers.push(sender);
        AssignedTaskWatch {
            initial: initial.into(),
            receiver,
        }
    }

    /// Ends every watch, because the worker quits.
    pub(crate) fn shut_down(&self) {
        self.shared().watchers.clear();
    }
}

/// The master's view of a worker's assigned tasks: the state of each active
/// task, then every change of any assigned task, in the order the worker made
/// them. A state carries the version of its task's latest stored state.
#[derive(Debug)]
pub struct AssignedTaskWatch {
    initial: VecDeque<AssignedTaskState>,
    receiver: mpsc::Receiver<AssignedTaskState>,
}

impl AssignedTaskWatch {
    /// The next state, or `None` once the watch ended: the watcher fell
    /// behind, or the worker quits. The master then watches again and asks
    /// for each task that it still follows.
    pub async fn next(&mut self) -> Option<AssignedTaskState> {
        if let Some(state) = self.initial.pop_front() {
            return Some(state);
        }
        self.receiver.recv().await
    }
}

/// Waits until a run ended. The wait owns its receiver and drops the borrow
/// of the watch at once, so the future can move between threads.
async fn run_ended(mut ended: watch::Receiver<bool>) -> bool {
    ended.wait_for(|ended| *ended).await.is_ok()
}

/// The state of an assigned task from its stored row, for a task that is not
/// live: it ended, or Mannie ended while it ran.
fn stored_state(task: &StoredTask) -> AssignedTaskState {
    let ending = task.ending.as_ref();
    AssignedTaskState {
        task_id: task.id.clone(),
        conversation_id: task.conversation_id.clone(),
        version: task.state_version,
        state: task.state,
        started_at: None,
        activity: None,
        tool_calls: 0,
        approval: None,
        waits_for_lease: false,
        report: ending.and_then(|ending| ending.report.clone()),
        failure: ending.and_then(|ending| ending.failure.clone()),
        ended_at: task.ended_at,
    }
}

fn record_of(task: &StoredTask) -> Option<AssignmentRecord> {
    let text = task.assignment.as_deref()?;
    match serde_json::from_str(text) {
        Ok(record) => Some(record),
        Err(error) => {
            tracing::error!(%error, task = %task.id, "the stored assignment of a task is not valid");
            None
        }
    }
}

impl Runtime {
    /// The agent that runs assigned tasks, on a machine that takes them.
    fn require_worker(&self) -> Result<&'static str, AssignError> {
        self.machine().assigned_agent.ok_or(AssignError::NotAWorker)
    }

    /// The state of an assigned task: live while its run works, from its row
    /// otherwise.
    fn assigned_state(&self, task: &StoredTask) -> AssignedTaskState {
        self.inner
            .assigned
            .get(&task.id)
            .unwrap_or_else(|| stored_state(task))
    }

    /// Checks what can be checked of an assignment before anything is
    /// stored: the instruction, the title and the profile.
    fn check_assignment(&self, assignment: &TaskAssignment) -> Result<String, AssignError> {
        check_instruction(&assignment.instruction).map_err(AssignError::invalid)?;
        let title = clean_label(&assignment.title, MAX_TITLE_CHARS)
            .ok_or_else(|| AssignError::invalid("the title is empty"))?;
        let profile = &assignment.profile;
        let base_url = check_base_url(&profile.base_url)
            .map_err(|error| AssignError::invalid(format!("the base URL is not valid: {error}")))?;
        if profile.has_credential && !credential_transport_allowed(&base_url) {
            return Err(AssignError::invalid(
                "a credential may not travel to this base URL without TLS",
            ));
        }
        if self.inner.registry.adapter(profile.interface).is_none() {
            return Err(AssignError::invalid(format!(
                "this worker does not speak the interface {}",
                profile.interface.as_str()
            )));
        }
        Ok(title)
    }

    /// Takes a task that the master assigned, and starts its run (ADR-0028,
    /// decision 1). The task and its turn, and its conversation when it is
    /// new, are stored in one batch. A task whose ID the worker knows is not
    /// assigned again: its state is returned, so the master can submit a
    /// task again after an answer it lost.
    pub async fn assign_task(
        &self,
        assignment: TaskAssignment,
    ) -> Result<AssignedTaskState, AssignError> {
        let agent = self.require_worker()?;
        if self.is_shutting_down() {
            return Err(AssignError::ShuttingDown);
        }
        let title = self.check_assignment(&assignment)?;
        let store = &self.inner.store;
        // One assignment at a time, so the same task submitted twice at once
        // is stored once.
        let _assigning = self.inner.assigning.lock().await;
        if let Some(stored) = store
            .task(&assignment.task_id)
            .await
            .map_err(|error| storage_failure(&error))?
        {
            if stored.conversation_id != assignment.conversation_id || stored.assignment.is_none() {
                return Err(AssignError::Conflict);
            }
            return Ok(self.assigned_state(&stored));
        }
        self.check_conversation(&assignment).await?;
        let conversation = assignment.conversation_id.clone();
        let reservation = self
            .reserve(&conversation, Busy::Starting)
            .map_err(|_busy| AssignError::StillRunning)?;
        let (changes, turn, run) = self.assignment_batch(&assignment, title, agent)?;
        self.inner
            .hubs
            .apply(store, &conversation, changes, Durability::Full)
            .await
            .map_err(|error| match error {
                RuntimeStoreError::RunActive | RuntimeStoreError::TaskActive => {
                    AssignError::StillRunning
                }
                RuntimeStoreError::ConversationExists => AssignError::Conflict,
                RuntimeStoreError::ConversationNotFound => AssignError::ConversationNotFound,
                other => storage_failure(&other),
            })?;
        let stored = store
            .task(&assignment.task_id)
            .await
            .map_err(|error| storage_failure(&error))?
            .ok_or(AssignError::StorageFailed)?;
        let assigned = &self.inner.assigned;
        assigned.accepted(&stored.id, &conversation, stored.state_version);
        let guard = ChildGuard::new(
            TaskSink::Assigned(Arc::clone(assigned)),
            stored.id.clone(),
            Arc::clone(&self.inner.clock),
        );
        let _mailbox = self.spawn_run(
            conversation.clone(),
            turn,
            run.id,
            run.profile_id,
            run.agent,
            Some(ChildLink {
                parent: None,
                guard,
                assigned: Some(AssignedRun {
                    task: stored.id.clone(),
                    profile: assignment.profile,
                    read_only: assignment.read_only,
                }),
            }),
        );
        // The run holds the conversation now.
        drop(reservation);
        tracing::info!(task = %stored.id, conversation = %conversation, follow_up = assignment.follow_up, read_only = assignment.read_only, "the worker took a task");
        Ok(self.assigned_state(&stored))
    }

    /// The batch that stores an assigned task: its conversation when it is
    /// new, and the turn with the instruction, the queued run of the worker
    /// agent with the task's profile, and the task with its assignment.
    /// Returns the turn and the run too.
    fn assignment_batch(
        &self,
        assignment: &TaskAssignment,
        title: String,
        agent: &str,
    ) -> Result<(Vec<Change>, TurnId, NewRun), AssignError> {
        let record = AssignmentRecord {
            profile: assignment.profile.clone(),
            read_only: assignment.read_only,
        };
        let record = serde_json::to_string(&record).map_err(|error| {
            tracing::error!(%error, "an assignment could not be written as JSON");
            AssignError::StorageFailed
        })?;
        let ids = &self.inner.ids;
        let turn = ids.turn();
        let run = NewRun {
            id: ids.run(),
            agent: agent.to_owned(),
            profile_id: assignment.profile.id.clone(),
            binding: assignment.profile.binding(),
        };
        let mut changes = Vec::with_capacity(2);
        if !assignment.follow_up {
            changes.push(Change::CreateConversation {
                conversation: NewConversation {
                    id: assignment.conversation_id.clone(),
                    title,
                    profile_id: None,
                    project_id: None,
                    task: None,
                },
            });
        }
        changes.push(Change::StartTurn {
            turn_id: turn.clone(),
            user_part_id: ids.part(),
            mentions: Vec::new(),
            images: Vec::new(),
            text: assignment.instruction.clone(),
            run: run.clone(),
            task: Some(NewTask {
                id: assignment.task_id.clone(),
                part_id: None,
                assignment: Some(record),
                follow_up: assignment.follow_up,
                background: false,
            }),
        });
        Ok((changes, turn, run))
    }

    /// Checks the conversation of a new task: a new task starts a new
    /// conversation, and a follow-up continues one of an assigned task that
    /// ended.
    async fn check_conversation(&self, assignment: &TaskAssignment) -> Result<(), AssignError> {
        let store = &self.inner.store;
        let conversation = &assignment.conversation_id;
        let exists = store
            .conversation_summary(conversation)
            .await
            .map_err(|error| storage_failure(&error))?
            .is_some();
        match (assignment.follow_up, exists) {
            (false, true) => Err(AssignError::Conflict),
            (true, false) => Err(AssignError::ConversationNotFound),
            (false, false) => Ok(()),
            (true, true) => {
                let latest = store
                    .latest_task(conversation)
                    .await
                    .map_err(|error| storage_failure(&error))?;
                match latest {
                    Some(task) if task.assignment.is_none() => {
                        Err(AssignError::ConversationNotFound)
                    }
                    Some(task) if task.state.is_active() => Err(AssignError::StillRunning),
                    None => Err(AssignError::ConversationNotFound),
                    Some(_ended) => Ok(()),
                }
            }
        }
    }

    /// The state of an assigned task, or `None` for a task that the worker
    /// does not know.
    pub async fn assigned_task(
        &self,
        task: &TaskId,
    ) -> Result<Option<AssignedTaskState>, AssignError> {
        self.require_worker()?;
        let stored = self
            .inner
            .store
            .task(task)
            .await
            .map_err(|error| storage_failure(&error))?;
        Ok(stored
            .filter(|stored| stored.assignment.is_some())
            .map(|stored| self.assigned_state(&stored)))
    }

    /// Follows the assigned tasks: the states of the active ones, then every
    /// change.
    pub fn watch_assigned_tasks(&self) -> AssignedTaskWatch {
        self.inner
            .assigned
            .watch(self.inner.config.subscriptions.queue)
    }

    /// Cancels an assigned task: its run stops, and the task ends as
    /// canceled. Waits for the run to end up to the stop timeout, and
    /// returns the task's state then, or `None` for a task that the worker
    /// does not know.
    pub async fn cancel_assigned_task(
        &self,
        task: &TaskId,
    ) -> Result<Option<AssignedTaskState>, AssignError> {
        self.require_worker()?;
        let store = &self.inner.store;
        let Some(stored) = store
            .task(task)
            .await
            .map_err(|error| storage_failure(&error))?
            .filter(|stored| stored.assignment.is_some())
        else {
            return Ok(None);
        };
        if stored.state.is_active() {
            tracing::info!(task = %task, "the master canceled a task");
            self.stop_reply(&stored.conversation_id).await;
        }
        let current = store
            .task(task)
            .await
            .map_err(|error| storage_failure(&error))?
            .unwrap_or(stored);
        Ok(Some(self.assigned_state(&current)))
    }

    /// Lets every assigned task go, because the worker is unpaired: each run
    /// stops, and its task ends as canceled. Waits for the runs up to the
    /// shutdown timeout.
    pub async fn release_assignments(&self) {
        let conversations = self.inner.assigned.active_conversations();
        let mut ended: Vec<watch::Receiver<bool>> = Vec::new();
        {
            let busy = lock(&self.inner.busy);
            for conversation in &conversations {
                if let Some(Busy::Running(handle)) = busy.get(conversation) {
                    // A full mailbox already holds a command that ends the
                    // run.
                    let _delivered = handle.mailbox.try_send(Command::Stop);
                    ended.push(handle.ended.clone());
                }
            }
        }
        tracing::info!(tasks = conversations.len(), "the worker lets its tasks go");
        let all_ended = futures_util::future::join_all(ended.into_iter().map(run_ended));
        if tokio::time::timeout(self.inner.config.shutdown_timeout, all_ended)
            .await
            .is_err()
        {
            tracing::warn!("the runs of released tasks did not end in time");
        }
    }

    /// Deletes conversations of assigned tasks that the master deleted with
    /// their parent (plugins/workers/docs/delegation.md, deleting). A
    /// running task stops first; a conversation that the worker does not
    /// have is skipped.
    pub async fn delete_assigned_conversations(
        &self,
        conversations: &[ConversationId],
    ) -> Result<(), AssignError> {
        self.require_worker()?;
        let store = &self.inner.store;
        for conversation in conversations {
            let latest = store
                .latest_task(conversation)
                .await
                .map_err(|error| storage_failure(&error))?;
            if latest.is_none_or(|task| task.assignment.is_none()) {
                continue;
            }
            self.stop_reply(conversation).await;
            let deleted = store
                .delete_conversation(conversation)
                .await
                .map_err(|error| storage_failure(&error))?;
            if deleted {
                self.inner.hubs.ended_task_conversation(conversation).await;
                tracing::info!(conversation = %conversation, "deleted the conversation of an assigned task");
            }
        }
        Ok(())
    }

    /// Keeps the credential that the master leases to an active task, in the
    /// vault, for the task's runs (plugins/workers/docs/credential-
    /// leases.md). A lease whose binding is not the task's profile's is
    /// refused.
    pub async fn deliver_lease(
        &self,
        task: &TaskId,
        binding: &str,
        credential: Secret,
    ) -> Result<(), LeaseError> {
        let store = &self.inner.store;
        let stored = store
            .task(task)
            .await
            .map_err(|error| lease_storage_failure(&error))?
            .filter(|stored| stored.state.is_active())
            .ok_or(LeaseError::TaskNotActive)?;
        let record = record_of(&stored).ok_or(LeaseError::TaskNotActive)?;
        if record.profile.binding() != binding {
            return Err(LeaseError::BindingMismatch);
        }
        let reference = lease_reference(task);
        in_vault(&self.inner.vault, move |vault| {
            vault.save(&reference, &credential)
        })
        .await
        .map_err(|_unavailable| LeaseError::VaultUnavailable)?;
        // A task that ended meanwhile no longer needs its lease, and its run
        // already dropped what it had.
        let still_active = store
            .task(task)
            .await
            .map_err(|error| lease_storage_failure(&error))?
            .is_some_and(|stored| stored.state.is_active());
        if !still_active {
            self.drop_lease(task).await;
            return Err(LeaseError::TaskNotActive);
        }
        tracing::info!(task = %task, "the worker keeps a credential lease");
        Ok(())
    }

    /// Drops a task's lease because the master revoked it. A run of the task
    /// keeps its own copy of the credential while it works, so the run ends
    /// too, as failed.
    pub async fn revoke_lease(&self, task: &TaskId) -> Result<(), LeaseError> {
        let reference = lease_reference(task);
        match in_vault(&self.inner.vault, move |vault| vault.delete(&reference)).await {
            Ok(()) | Err(VaultError::Missing) => {}
            Err(VaultError::Unavailable(_problem)) => return Err(LeaseError::VaultUnavailable),
        }
        let stored = self
            .inner
            .store
            .task(task)
            .await
            .map_err(|error| lease_storage_failure(&error))?;
        if let Some(stored) = stored.filter(|stored| stored.state.is_active()) {
            let busy = lock(&self.inner.busy);
            if let Some(Busy::Running(handle)) = busy.get(&stored.conversation_id) {
                // A full mailbox already holds a command that ends the run.
                let _delivered = handle.mailbox.try_send(Command::LeaseRevoked);
            }
            tracing::info!(task = %task, "the master revoked a running task's lease; its run ends");
        }
        Ok(())
    }

    /// Drops a task's lease from the vault, once the task no longer needs
    /// it. A lease that is not there is fine.
    pub(crate) async fn drop_lease(&self, task: &TaskId) {
        let reference = lease_reference(task);
        match in_vault(&self.inner.vault, move |vault| vault.delete(&reference)).await {
            Ok(()) | Err(VaultError::Missing) => {}
            Err(VaultError::Unavailable(problem)) => {
                tracing::warn!(task = %task, problem = ?problem, "could not drop the lease of a task that ended");
            }
        }
    }
}
