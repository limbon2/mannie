//! Delegating tasks to workers (ADR-0027; docs/features/workers/
//! delegation.md): the port through which the runtime reaches the workers,
//! which the master's composition root gives it, and the runtime's side of
//! following each worker's tasks.
//!
//! The runtime follows each paired worker while the link to it works: one
//! watch of the states of its tasks, the endings that it missed while the
//! link was down, the leases that its tasks wait for, and the cancels and
//! deletions that wait for it. Each state reaches the row of its task, and
//! the board of the parent's run that follows the task, as a child's state
//! does.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use mannie_store::{RuntimeStoreError, StoredTask, Submission, TaskEnding};
use mannie_types::chat::{ChatError, CheckpointRef, EarlierTurns, LoadEarlierTurns};
use mannie_types::ids::{ConversationId, TaskId, WorkerId};
use mannie_types::secret::Secret;
use mannie_types::tasks::{AssignedTaskState, ChildApproval, TaskState};
use mannie_types::tools::{
    ApprovalChoice, DecideApproval, ReadToolOutput, ToolOutputPage, ToolsError,
};
use mannie_types::transcript::{Checkpoint, ConversationSummary, ConversationUpdate};
use mannie_types::workers::{TaskAssignment, WorkerStatus, WorkerView};
use tokio::sync::watch;

use crate::Runtime;
use crate::profiles::in_vault;
use crate::tasks::{ChildEnd, TaskBoard};

/// How long a follower waits before it watches a connected worker again
/// after its watch failed.
const FOLLOW_RETRY: Duration = Duration::from_secs(1);

/// Why a worker did not do what the runtime asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DelegationError {
    /// The worker cannot be reached now.
    #[error("the worker cannot be reached")]
    Unreachable,
    /// The worker refused, with its error's stable code.
    #[error("the worker refused: {code}")]
    Refused {
        /// The worker's code.
        code: String,
    },
}

/// What the runtime asks of the workers (ADR-0027, decision 1). The master's
/// composition root gives it to the runtime, and `mannie-link` implements it
/// over the link; a worker's runtime has none. Every call may fail with the
/// worker unreachable, and the calls that the link carries to the worker's
/// runtime answer with that runtime's errors.
pub trait Delegation: Send + Sync + fmt::Debug + 'static {
    /// The paired workers, as they are now and as they change.
    fn workers(&self) -> watch::Receiver<Vec<WorkerView>>;

    /// Submits a task to a worker, which stores it once, whatever happens to
    /// the answer: a task that the worker knows is returned as it is.
    fn submit(
        &self,
        worker: &WorkerId,
        assignment: TaskAssignment,
    ) -> BoxFuture<'static, Result<AssignedTaskState, DelegationError>>;

    /// A task's state on its worker, or `None` for a task that the worker
    /// does not know.
    fn task(
        &self,
        worker: &WorkerId,
        task: &TaskId,
    ) -> BoxFuture<'static, Result<Option<AssignedTaskState>, DelegationError>>;

    /// Asks a worker to cancel a task, and returns the task's state then.
    fn cancel(
        &self,
        worker: &WorkerId,
        task: &TaskId,
    ) -> BoxFuture<'static, Result<Option<AssignedTaskState>, DelegationError>>;

    /// Follows the states of a worker's tasks: the state of each active task,
    /// then every change. The stream ends when the link drops.
    fn watch(
        &self,
        worker: &WorkerId,
    ) -> BoxFuture<'static, Result<BoxStream<'static, AssignedTaskState>, DelegationError>>;

    /// Delivers the credential lease of a task.
    fn deliver_lease(
        &self,
        worker: &WorkerId,
        task: &TaskId,
        binding: String,
        credential: Secret,
    ) -> BoxFuture<'static, Result<(), DelegationError>>;

    /// Revokes the credential lease of a task.
    fn revoke_lease(
        &self,
        worker: &WorkerId,
        task: &TaskId,
    ) -> BoxFuture<'static, Result<(), DelegationError>>;

    /// Deletes conversations of tasks on a worker.
    fn delete_conversations(
        &self,
        worker: &WorkerId,
        conversations: Vec<ConversationId>,
    ) -> BoxFuture<'static, Result<(), DelegationError>>;

    /// Follows a task conversation on its worker, from a cursor or from a
    /// snapshot. The stream ends when the link drops.
    fn subscribe(
        &self,
        worker: &WorkerId,
        conversation: &ConversationId,
        after_seq: Option<u32>,
    ) -> BoxFuture<'static, Result<BoxStream<'static, ConversationUpdate>, ChatError>>;

    /// Earlier turns of a task conversation on its worker.
    fn earlier_turns(
        &self,
        worker: &WorkerId,
        input: LoadEarlierTurns,
    ) -> BoxFuture<'static, Result<EarlierTurns, ChatError>>;

    /// A checkpoint of a task conversation on its worker.
    fn checkpoint(
        &self,
        worker: &WorkerId,
        input: CheckpointRef,
    ) -> BoxFuture<'static, Result<Checkpoint, ChatError>>;

    /// A page of an archived output of a task conversation on its worker.
    fn tool_output(
        &self,
        worker: &WorkerId,
        input: ReadToolOutput,
    ) -> BoxFuture<'static, Result<ToolOutputPage, ToolsError>>;

    /// The owner's decision about a call of a task on its worker.
    fn decide_approval(
        &self,
        worker: &WorkerId,
        input: DecideApproval,
    ) -> BoxFuture<'static, Result<(), ToolsError>>;

    /// Allows once a call of a task on its worker, because the task's parent
    /// conversation is in full access (ADR-0029, decision 5). A worker whose
    /// configuration does not take such answers refuses with
    /// `full_access_refused`, and the call keeps waiting for the owner.
    fn allow_by_full_access(
        &self,
        worker: &WorkerId,
        input: DecideApproval,
    ) -> BoxFuture<'static, Result<(), ToolsError>>;
}

/// What the runtime keeps to follow the workers' tasks.
#[derive(Debug)]
pub(crate) struct Followers {
    /// The boards of the runs that follow delegated tasks, by task.
    boards: Mutex<HashMap<TaskId, Arc<TaskBoard>>>,
    /// The tasks whose submission a run is making now, which a follower's
    /// comparison with the worker leaves alone.
    submitting: Mutex<HashSet<TaskId>>,
    /// The tasks whose lease was delivered or is being delivered: the run
    /// that submits a task and the follower of its worker both offer the
    /// lease, and the worker gets it once. A failed delivery and the task's
    /// end take the task out again.
    leases: Mutex<HashSet<TaskId>>,
    /// Counts the cancels and deletions that were queued, so a follower
    /// sends them while its worker is connected.
    queued: watch::Sender<u64>,
    /// Tells the followers that the runtime shuts down.
    shutdown: watch::Sender<bool>,
}

impl Default for Followers {
    fn default() -> Self {
        Self {
            boards: Mutex::default(),
            submitting: Mutex::default(),
            leases: Mutex::default(),
            queued: watch::Sender::new(0),
            shutdown: watch::Sender::new(false),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every critical section only inserts or removes entries, so the data
    // stays consistent even if a holder panicked.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Followers {
    fn board(&self, task: &TaskId) -> Option<Arc<TaskBoard>> {
        lock(&self.boards).get(task).map(Arc::clone)
    }

    fn is_submitting(&self, task: &TaskId) -> bool {
        lock(&self.submitting).contains(task)
    }

    /// Takes on the delivery of a task's lease; false when the lease was
    /// delivered or is being delivered.
    fn claim_lease(&self, task: &TaskId) -> bool {
        lock(&self.leases).insert(task.clone())
    }

    /// Lets the next state that waits for a task's lease deliver it again.
    pub(crate) fn forget_lease(&self, task: &TaskId) {
        lock(&self.leases).remove(task);
    }

    /// Wakes the followers to send what was queued.
    pub(crate) fn nudge(&self) {
        self.queued
            .send_modify(|count| *count = count.wrapping_add(1));
    }

    /// Ends every follower.
    pub(crate) fn shut_down(&self) {
        self.shutdown.send_replace(true);
    }
}

/// A run's hold on the states of a delegated task: while it lives, the
/// followers publish the task's states on the run's board.
#[derive(Debug)]
pub(crate) struct RemoteFollow {
    runtime: Runtime,
    task: TaskId,
}

impl RemoteFollow {
    /// Follows a task on `board` from now on.
    pub(crate) fn new(runtime: &Runtime, task: &TaskId, board: &Arc<TaskBoard>) -> Self {
        lock(&runtime.inner.followers.boards).insert(task.clone(), Arc::clone(board));
        Self {
            runtime: runtime.clone(),
            task: task.clone(),
        }
    }
}

impl Drop for RemoteFollow {
    fn drop(&mut self) {
        lock(&self.runtime.inner.followers.boards).remove(&self.task);
    }
}

/// A run's submission of a task, while it is made: a follower that compares
/// its rows with the worker leaves the task alone.
#[derive(Debug)]
pub(crate) struct Submitting {
    runtime: Runtime,
    task: TaskId,
}

impl Submitting {
    pub(crate) fn new(runtime: &Runtime, task: &TaskId) -> Self {
        lock(&runtime.inner.followers.submitting).insert(task.clone());
        Self {
            runtime: runtime.clone(),
            task: task.clone(),
        }
    }
}

impl Drop for Submitting {
    fn drop(&mut self) {
        lock(&self.runtime.inner.followers.submitting).remove(&self.task);
    }
}

/// The status of a worker in the list, or `None` once it was removed.
fn status_of(workers: &[WorkerView], worker: &WorkerId) -> Option<WorkerStatus> {
    workers
        .iter()
        .find(|view| view.id == *worker)
        .map(|view| view.status)
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
    /// The port to the workers, on the master.
    pub(crate) fn delegation(&self) -> Option<&Arc<dyn Delegation>> {
        self.inner.delegation.as_ref()
    }

    /// The worker of a task conversation that runs on one, with the master's
    /// summary of the conversation; `None` for every other conversation.
    pub(crate) async fn remote_conversation(
        &self,
        conversation: &ConversationId,
    ) -> Result<Option<(Arc<dyn Delegation>, WorkerId, ConversationSummary)>, RuntimeStoreError>
    {
        let Some(delegation) = self.delegation().map(Arc::clone) else {
            return Ok(None);
        };
        let summary = self.inner.store.conversation_summary(conversation).await?;
        Ok(summary.and_then(|summary| {
            let worker = summary.task.as_ref()?.worker.as_ref()?.id.clone();
            Some((delegation, worker, summary))
        }))
    }

    /// Cancels the active task of a conversation on a worker, as Stop in
    /// the conversation does: the cancel is stored and sent while the worker
    /// is connected. Returns whether a task was active.
    pub(crate) async fn stop_remote_task(&self, conversation: &ConversationId) -> bool {
        let latest = match self.inner.store.latest_task(conversation).await {
            Ok(latest) => latest,
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not read the task of a conversation");
                return false;
            }
        };
        let Some(task) = latest.filter(|task| task.state.is_active() && task.worker_id.is_some())
        else {
            return false;
        };
        match self.inner.store.request_task_cancel(&task.id).await {
            Ok(active) => {
                self.inner.followers.nudge();
                tracing::info!(task = %task.id, "the owner stopped a task on a worker");
                active
            }
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), task = %task.id, "could not store the cancel of a task on a worker");
                false
            }
        }
    }

    /// The paired workers as they are now; none without a port.
    pub(crate) fn workers_now(&self) -> Vec<WorkerView> {
        self.delegation()
            .map(|delegation| delegation.workers().borrow().clone())
            .unwrap_or_default()
    }

    /// Starts following the paired workers, and each worker paired later.
    pub(crate) fn start_following(&self) {
        let Some(delegation) = self.delegation().map(Arc::clone) else {
            return;
        };
        let runtime = self.clone();
        tokio::spawn(async move { runtime.supervise_followers(delegation).await });
    }

    /// Keeps one follower for each paired worker.
    async fn supervise_followers(&self, delegation: Arc<dyn Delegation>) {
        let mut workers = delegation.workers();
        let mut shutdown = self.inner.followers.shutdown.subscribe();
        let mut following: HashMap<WorkerId, tokio::task::JoinHandle<()>> = HashMap::new();
        loop {
            following.retain(|_worker, follower| !follower.is_finished());
            let ids: Vec<WorkerId> = workers
                .borrow_and_update()
                .iter()
                .map(|view| view.id.clone())
                .collect();
            for id in ids {
                following.entry(id).or_insert_with_key(|worker| {
                    let runtime = self.clone();
                    let delegation = Arc::clone(&delegation);
                    let worker = worker.clone();
                    tokio::spawn(async move { runtime.follow_worker(delegation, worker).await })
                });
            }
            if *shutdown.borrow_and_update() {
                return;
            }
            tokio::select! {
                changed = workers.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
                _shutdown = shutdown.changed() => return,
            }
        }
    }

    /// Follows one worker until it is removed or the runtime shuts down.
    async fn follow_worker(&self, delegation: Arc<dyn Delegation>, worker: WorkerId) {
        let mut workers = delegation.workers();
        let mut shutdown = self.inner.followers.shutdown.subscribe();
        loop {
            if *shutdown.borrow_and_update() {
                return;
            }
            let status = status_of(&workers.borrow_and_update(), &worker);
            match status {
                None => {
                    self.forget_worker_tasks(&worker).await;
                    return;
                }
                Some(WorkerStatus::Connected) => {
                    match delegation.watch(&worker).await {
                        Ok(stream) => {
                            self.follow_connection(
                                &delegation,
                                &worker,
                                stream,
                                &mut workers,
                                &mut shutdown,
                            )
                            .await;
                        }
                        Err(error) => {
                            tracing::debug!(worker = %worker, %error, "could not watch a worker's tasks");
                        }
                    }
                    tokio::select! {
                        () = tokio::time::sleep(FOLLOW_RETRY) => {}
                        _shutdown = shutdown.changed() => return,
                    }
                }
                Some(_waiting) => {
                    tokio::select! {
                        changed = workers.changed() => {
                            if changed.is_err() {
                                return;
                            }
                        }
                        _shutdown = shutdown.changed() => return,
                    }
                }
            }
        }
    }

    /// Follows a worker's tasks while the link works: first what happened
    /// while it was down, then every state, and what waits for the worker.
    async fn follow_connection(
        &self,
        delegation: &Arc<dyn Delegation>,
        worker: &WorkerId,
        mut stream: BoxStream<'static, AssignedTaskState>,
        workers: &mut watch::Receiver<Vec<WorkerView>>,
        shutdown: &mut watch::Receiver<bool>,
    ) {
        tracing::info!(worker = %worker, "following the tasks of a worker");
        let mut queued = self.inner.followers.queued.subscribe();
        if self.reconcile(delegation, worker).await.is_err() {
            return;
        }
        loop {
            tokio::select! {
                state = stream.next() => match state {
                    Some(state) => self.receive_state(delegation, worker, state).await,
                    None => return,
                },
                changed = queued.changed() => {
                    if changed.is_err() || self.send_queued(delegation, worker).await.is_err() {
                        return;
                    }
                }
                changed = workers.changed() => {
                    let connected = status_of(&workers.borrow_and_update(), worker)
                        == Some(WorkerStatus::Connected);
                    if changed.is_err() || !connected {
                        return;
                    }
                }
                _shutdown = shutdown.changed() => return,
            }
        }
    }

    /// Compares the master's active tasks of a worker with the worker's,
    /// after a connection: a task that the worker does not know ends as
    /// interrupted, and nothing submits it again (ADR-0027, decision 7).
    /// Then sends what waits for the worker.
    async fn reconcile(
        &self,
        delegation: &Arc<dyn Delegation>,
        worker: &WorkerId,
    ) -> Result<(), DelegationError> {
        let active = match self.inner.store.active_delegated_tasks().await {
            Ok(active) => active,
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not read the tasks of a worker");
                return Ok(());
            }
        };
        for task in active
            .iter()
            .filter(|task| task.worker_id.as_ref() == Some(worker))
        {
            if self.inner.followers.is_submitting(&task.id) {
                continue;
            }
            if let Some(state) = delegation.task(worker, &task.id).await? {
                self.receive_state(delegation, worker, state).await;
            } else {
                tracing::info!(task = %task.id, worker = %worker, "the worker does not know a task; it ends as interrupted");
                self.end_remote_task(task, TaskState::Interrupted).await;
            }
        }
        self.send_queued(delegation, worker).await
    }

    /// Sends the cancels and the deletions that wait for a worker.
    async fn send_queued(
        &self,
        delegation: &Arc<dyn Delegation>,
        worker: &WorkerId,
    ) -> Result<(), DelegationError> {
        let store = &self.inner.store;
        if let Ok(active) = store.active_delegated_tasks().await {
            for task in active.iter().filter(|task| {
                task.worker_id.as_ref() == Some(worker) && task.cancel_requested_at.is_some()
            }) {
                if let Some(state) = delegation.cancel(worker, &task.id).await? {
                    self.receive_state(delegation, worker, state).await;
                }
            }
        }
        let deletions = match store.worker_deletions(worker).await {
            Ok(deletions) => deletions,
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not read the deletions that wait for a worker");
                return Ok(());
            }
        };
        if deletions.is_empty() {
            return Ok(());
        }
        delegation
            .delete_conversations(worker, deletions.clone())
            .await?;
        if let Err(error) = store.drop_worker_deletions(worker, &deletions).await {
            tracing::warn!(%error, error.code = error.code(), "could not forget deletions that reached their worker");
        }
        tracing::info!(worker = %worker, conversations = deletions.len(), "a worker deleted conversations of its tasks");
        Ok(())
    }

    /// Takes a state that a worker published: stores it in the task's row,
    /// unless the row has a newer one, publishes it on the board of the run
    /// that follows the task, and delivers the task's lease when its run
    /// waits for one. A worker's state of a task that is not its own changes
    /// nothing.
    pub(crate) async fn receive_state(
        &self,
        delegation: &Arc<dyn Delegation>,
        worker: &WorkerId,
        state: AssignedTaskState,
    ) {
        let store = &self.inner.store;
        let row = match store.task(&state.task_id).await {
            Ok(Some(row)) => row,
            Ok(None) => return,
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not read a delegated task");
                return;
            }
        };
        if row.worker_id.as_ref() != Some(worker) || row.conversation_id != state.conversation_id {
            tracing::warn!(task = %state.task_id, worker = %worker, "a worker sent the state of a task that is not its own");
            return;
        }
        if !row.state.is_active() {
            return;
        }
        if row
            .remote_version
            .is_some_and(|stored| state.version < stored)
        {
            return;
        }
        let ending = ending_of(&state);
        if let Err(error) = store
            .apply_remote_task_state(&row.id, state.version, state.state, ending.clone())
            .await
        {
            tracing::error!(%error, error.code = error.code(), task = %row.id, "could not store a worker's task state");
            return;
        }
        if row.submission == Some(Submission::Pending)
            && let Err(error) = store
                .set_task_submission(&row.id, Submission::Accepted)
                .await
        {
            tracing::warn!(%error, error.code = error.code(), task = %row.id, "could not mark a task as stored by its worker");
        }
        let ended = ending.is_some();
        if ended {
            self.inner.followers.forget_lease(&row.id);
        }
        let approval = state.approval.clone();
        self.publish_remote(&state, ending);
        if let Some(approval) = &approval
            && !ended
        {
            self.answer_by_full_access(delegation, worker, &row, approval)
                .await;
        }
        if ended && row.profile_id.is_some() {
            self.revoke_ended_lease(delegation, worker, &row.id).await;
        } else if state.waits_for_lease {
            self.lease_to(delegation, worker, &row).await;
        }
    }

    /// Answers a worker's request at once when the task's parent conversation
    /// is in full access and the worker takes such answers, as it says in its
    /// `Hello` (ADR-0029, decision 5). Otherwise the request waits for the
    /// owner in the task's card.
    async fn answer_by_full_access(
        &self,
        delegation: &Arc<dyn Delegation>,
        worker: &WorkerId,
        task: &StoredTask,
        approval: &ChildApproval,
    ) {
        let Some(request) = &approval.call.request else {
            return;
        };
        if !self.parent_has_full_access(&task.conversation_id).await {
            return;
        }
        let accepts = delegation
            .workers()
            .borrow()
            .iter()
            .find(|view| view.id == *worker)
            .and_then(|view| view.details.ceiling.as_ref())
            .is_none_or(|ceiling| ceiling.accepts_master_full_access);
        if !accepts {
            return;
        }
        let input = DecideApproval {
            conversation_id: task.conversation_id.clone(),
            part_id: approval.part_id.clone(),
            digest: request.digest.clone(),
            choice: ApprovalChoice::AllowOnce,
        };
        match delegation.allow_by_full_access(worker, input).await {
            Ok(()) => {
                tracing::info!(task = %task.id, worker = %worker, "full access allowed a call of a task on a worker");
            }
            // The owner answered first, or a state of the same request came
            // again; either way the call no longer waits for this answer.
            Err(ToolsError::ApprovalNotPending | ToolsError::ApprovalChanged) => {}
            Err(error) => {
                tracing::info!(error.code = error.code(), task = %task.id, worker = %worker, "the call of a task on a worker waits for the owner");
            }
        }
    }

    /// Whether the parent of a task conversation is in full access.
    async fn parent_has_full_access(&self, conversation: &ConversationId) -> bool {
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

    /// Revokes the lease of a task that ended. The worker drops a task's
    /// lease when the task ends, so this finds nothing to drop unless the
    /// worker missed that (docs/features/workers/credential-leases.md).
    async fn revoke_ended_lease(
        &self,
        delegation: &Arc<dyn Delegation>,
        worker: &WorkerId,
        task: &TaskId,
    ) {
        if let Err(error) = delegation.revoke_lease(worker, task).await {
            tracing::info!(%error, task = %task, worker = %worker, "could not revoke the lease of a task that ended");
        }
    }

    /// Publishes a worker's state of a task on the board of the run that
    /// follows it, if a run does.
    fn publish_remote(&self, state: &AssignedTaskState, ending: Option<TaskEnding>) {
        let Some(board) = self.inner.followers.board(&state.task_id) else {
            return;
        };
        let now = self.inner.clock.now();
        board.publish(&state.task_id, |child| {
            child.started_at = state
                .started_at
                .or(child.started_at)
                .or_else(|| (state.state != TaskState::Accepted).then_some(now));
            child.activity.clone_from(&state.activity);
            // A task's count only grows. The state of a task that ended
            // comes from the worker's stored row after a reconnect, and that
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

    /// Delivers a task's lease once: the credential of the task's profile,
    /// from the master's credential store (ADR-0010). A profile without a
    /// credential needs no lease. After a failed delivery, the next state
    /// that waits for the lease tries again.
    pub(crate) async fn lease_to(
        &self,
        delegation: &Arc<dyn Delegation>,
        worker: &WorkerId,
        task: &StoredTask,
    ) {
        let followers = &self.inner.followers;
        if !followers.claim_lease(&task.id) {
            return;
        }
        if !self.send_lease(delegation, worker, task).await {
            followers.forget_lease(&task.id);
        }
    }

    /// Sends a task's lease to its worker; false when the worker did not get
    /// a lease that the task needs.
    async fn send_lease(
        &self,
        delegation: &Arc<dyn Delegation>,
        worker: &WorkerId,
        task: &StoredTask,
    ) -> bool {
        let Some(profile_id) = &task.profile_id else {
            return true;
        };
        let profile = match self.inner.store.profile(profile_id).await {
            Ok(Some(profile)) => profile,
            Ok(None) => {
                tracing::warn!(task = %task.id, "the profile of a delegated task was deleted; its lease cannot be delivered");
                return false;
            }
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), "could not read the profile of a delegated task");
                return false;
            }
        };
        let Some(reference) = profile.credential_ref.clone() else {
            return true;
        };
        let credential = match in_vault(&self.inner.vault, move |vault| vault.read(&reference))
            .await
        {
            Ok(credential) => credential,
            Err(error) => {
                tracing::warn!(%error, task = %task.id, "could not read the credential of a delegated task");
                return false;
            }
        };
        match delegation
            .deliver_lease(worker, &task.id, profile.binding(), credential)
            .await
        {
            Ok(()) => {
                tracing::info!(task = %task.id, worker = %worker, "delivered a task's lease");
                true
            }
            Err(error) => {
                tracing::warn!(%error, task = %task.id, worker = %worker, "could not deliver a task's lease");
                false
            }
        }
    }

    /// Ends a delegated task on the master, as when its worker does not know
    /// it or was removed, and tells the run that follows it.
    async fn end_remote_task(&self, task: &StoredTask, state: TaskState) {
        let ending = TaskEnding {
            state,
            report: None,
            failure: None,
        };
        let version = task.remote_version.unwrap_or(0);
        if let Err(error) = self
            .inner
            .store
            .apply_remote_task_state(&task.id, version, state, Some(ending.clone()))
            .await
        {
            tracing::error!(%error, error.code = error.code(), task = %task.id, "could not end a delegated task");
            return;
        }
        self.inner.followers.forget_lease(&task.id);
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
        self.publish_remote(&ended, Some(ending));
    }

    /// Ends the active tasks of a worker that was removed: it canceled them
    /// when it was unpaired, or it can no longer report them.
    async fn forget_worker_tasks(&self, worker: &WorkerId) {
        let Ok(active) = self.inner.store.active_delegated_tasks().await else {
            return;
        };
        for task in active
            .iter()
            .filter(|task| task.worker_id.as_ref() == Some(worker))
        {
            self.end_remote_task(task, TaskState::Interrupted).await;
        }
        tracing::info!(worker = %worker, "stopped following a removed worker");
    }
}
