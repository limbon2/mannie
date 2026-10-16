//! Following the workers' tasks (ADR-0027, decisions 5 to 7; docs/features/
//! workers/delegation.md): while the link to a paired worker works, one
//! watch of the states of its tasks, the endings that it missed while the
//! link was down, the leases that its tasks wait for, and the cancels and
//! deletions that wait for it. Each state reaches the runtime, which stores
//! it in the task's row and shows it to the run that follows the task.

use std::collections::HashMap;
use std::time::Duration;

use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use mannie_runtime::LendError;
use mannie_store::StoredTask;
use mannie_types::ids::{TaskId, WorkerId};
use mannie_types::tasks::{AssignedTaskState, ChildApproval, TaskState};
use mannie_types::tools::{ApprovalChoice, DecideApproval, ToolsError};
use tokio::sync::watch;

use crate::delegation::DelegationError;
use crate::executor::{Shared, Submission, WORKERS_KIND, WorkerTaskDocument, WorkersExecutor};
use crate::view::{WorkerStatus, WorkerView};

/// How long a follower waits before it watches a connected worker again
/// after its watch failed.
const FOLLOW_RETRY: Duration = Duration::from_secs(1);

/// The status of a worker in the list, or `None` once it was removed.
fn status_of(workers: &[WorkerView], worker: &WorkerId) -> Option<WorkerStatus> {
    workers
        .iter()
        .find(|view| view.id == *worker)
        .map(|view| view.status)
}

/// The worker that runs a task, from the task's row.
fn worker_of(task: &StoredTask) -> Option<&str> {
    task.executor
        .as_ref()
        .filter(|execution| execution.kind == WORKERS_KIND)
        .map(|execution| execution.target.as_str())
}

/// The executor's document of a task's row.
fn document_of(task: &StoredTask) -> WorkerTaskDocument {
    let document = task
        .executor
        .as_ref()
        .and_then(|execution| execution.document.as_deref());
    WorkerTaskDocument::of(document, &task.id)
}

impl WorkersExecutor {
    /// Starts following the paired workers, and each worker paired later,
    /// until [`WorkersExecutor::stop`]. Turning Workers on compares the
    /// master's worker tasks with each worker's, as after a restart
    /// (ADR-0031, decision 10).
    pub fn start(&self) {
        let shared = std::sync::Arc::clone(&self.shared);
        tokio::spawn(async move {
            sweep_outbox(&shared).await;
            supervise_followers(shared).await;
        });
    }
}

/// Drops the deletions in the outbox for workers that are no longer paired,
/// which a removal could not drop because Mannie ended first.
async fn sweep_outbox(shared: &Shared) {
    let entries = match shared.services.executor_outbox(WORKERS_KIND).await {
        Ok(entries) => entries,
        Err(error) => {
            tracing::error!(%error, error.code = error.code(), "could not read the deletions that wait for workers");
            return;
        }
    };
    let paired: Vec<String> = shared
        .workers_now()
        .iter()
        .map(|view| view.id.as_str().to_owned())
        .collect();
    let stale: Vec<_> = entries
        .into_iter()
        .filter(|entry| !paired.contains(&entry.target))
        .collect();
    if stale.is_empty() {
        return;
    }
    if let Err(error) = shared
        .services
        .drop_executor_outbox(WORKERS_KIND, &stale)
        .await
    {
        tracing::error!(%error, error.code = error.code(), "could not drop the deletions of removed workers");
    }
}

/// Keeps one follower for each paired worker.
async fn supervise_followers(shared: std::sync::Arc<Shared>) {
    let mut workers = shared.delegation.workers();
    let mut stopping = shared.stopping.subscribe();
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
                let shared = std::sync::Arc::clone(&shared);
                let worker = worker.clone();
                tokio::spawn(async move { follow_worker(&shared, worker).await })
            });
        }
        if *stopping.borrow_and_update() {
            return;
        }
        tokio::select! {
            changed = workers.changed() => {
                if changed.is_err() {
                    return;
                }
            }
            _stopping = stopping.changed() => return,
        }
    }
}

/// Follows one worker until it is removed or the plugin stops.
async fn follow_worker(shared: &Shared, worker: WorkerId) {
    let mut workers = shared.delegation.workers();
    let mut stopping = shared.stopping.subscribe();
    loop {
        if *stopping.borrow_and_update() {
            return;
        }
        let status = status_of(&workers.borrow_and_update(), &worker);
        match status {
            None => {
                forget_worker_tasks(shared, &worker).await;
                return;
            }
            Some(WorkerStatus::Connected) => {
                match shared.delegation.watch(&worker).await {
                    Ok(stream) => {
                        follow_connection(shared, &worker, stream, &mut workers, &mut stopping)
                            .await;
                    }
                    Err(error) => {
                        tracing::debug!(worker = %worker, %error, "could not watch a worker's tasks");
                    }
                }
                tokio::select! {
                    () = tokio::time::sleep(FOLLOW_RETRY) => {}
                    _stopping = stopping.changed() => return,
                }
            }
            Some(_waiting) => {
                tokio::select! {
                    changed = workers.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                    _stopping = stopping.changed() => return,
                }
            }
        }
    }
}

/// Follows a worker's tasks while the link works: first what happened while
/// it was down, then every state, and what waits for the worker.
async fn follow_connection(
    shared: &Shared,
    worker: &WorkerId,
    mut stream: BoxStream<'static, AssignedTaskState>,
    workers: &mut watch::Receiver<Vec<WorkerView>>,
    stopping: &mut watch::Receiver<bool>,
) {
    tracing::info!(worker = %worker, "following the tasks of a worker");
    let mut queued = shared.queued.subscribe();
    let mut outbox = shared.services.outbox_changes();
    if reconcile(shared, worker).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            state = stream.next() => match state {
                Some(state) => receive_state(shared, worker, state).await,
                None => return,
            },
            changed = queued.changed() => {
                if changed.is_err() || send_queued(shared, worker).await.is_err() {
                    return;
                }
            }
            changed = outbox.changed() => {
                if changed.is_err() || send_queued(shared, worker).await.is_err() {
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
            _stopping = stopping.changed() => return,
        }
    }
}

/// Compares the master's active tasks of a worker with the worker's, after a
/// connection: a task that the worker does not know ends as interrupted, and
/// nothing submits it again (ADR-0027, decision 7). Then sends what waits for
/// the worker.
async fn reconcile(shared: &Shared, worker: &WorkerId) -> Result<(), DelegationError> {
    let active = match shared.services.active_executor_tasks(WORKERS_KIND).await {
        Ok(active) => active,
        Err(error) => {
            tracing::error!(%error, error.code = error.code(), "could not read the tasks of a worker");
            return Ok(());
        }
    };
    for task in active
        .iter()
        .filter(|task| worker_of(task) == Some(worker.as_str()))
    {
        if shared.is_submitting(&task.id) {
            continue;
        }
        if let Some(state) = shared.delegation.task(worker, &task.id).await? {
            receive_state(shared, worker, state).await;
        } else {
            tracing::info!(task = %task.id, worker = %worker, "the worker does not know a task; it ends as interrupted");
            end_remote_task(shared, task, TaskState::Interrupted).await;
        }
    }
    send_queued(shared, worker).await
}

/// Sends the cancels and the deletions that wait for a worker.
async fn send_queued(shared: &Shared, worker: &WorkerId) -> Result<(), DelegationError> {
    let services = &shared.services;
    if let Ok(active) = services.active_executor_tasks(WORKERS_KIND).await {
        for task in active.iter().filter(|task| {
            worker_of(task) == Some(worker.as_str())
                && document_of(task).cancel_requested_at.is_some()
        }) {
            if let Some(state) = shared.delegation.cancel(worker, &task.id).await? {
                receive_state(shared, worker, state).await;
            }
        }
    }
    let entries = match services.executor_outbox(WORKERS_KIND).await {
        Ok(entries) => entries,
        Err(error) => {
            tracing::error!(%error, error.code = error.code(), "could not read the deletions that wait for a worker");
            return Ok(());
        }
    };
    let deletions: Vec<_> = entries
        .into_iter()
        .filter(|entry| entry.target == worker.as_str())
        .collect();
    if deletions.is_empty() {
        return Ok(());
    }
    let conversations = deletions
        .iter()
        .map(|entry| entry.conversation_id.clone())
        .collect();
    shared
        .delegation
        .delete_conversations(worker, conversations)
        .await?;
    if let Err(error) = services
        .drop_executor_outbox(WORKERS_KIND, &deletions)
        .await
    {
        tracing::warn!(%error, error.code = error.code(), "could not forget deletions that reached their worker");
    }
    tracing::info!(worker = %worker, conversations = deletions.len(), "a worker deleted conversations of its tasks");
    Ok(())
}

/// Takes a state that a worker published: the runtime stores it in the
/// task's row, unless the row has a newer one, and shows it to the run that
/// follows the task; the follower then delivers the task's lease when its
/// run waits for one. A worker's state of a task that is not its own changes
/// nothing.
async fn receive_state(shared: &Shared, worker: &WorkerId, state: AssignedTaskState) {
    let services = &shared.services;
    let row = match services.executor_task(&state.task_id).await {
        Ok(Some(row)) => row,
        Ok(None) => return,
        Err(error) => {
            tracing::error!(%error, error.code = error.code(), "could not read a worker task");
            return;
        }
    };
    if worker_of(&row) != Some(worker.as_str()) {
        tracing::warn!(task = %state.task_id, worker = %worker, "a worker sent the state of a task that is not its own");
        return;
    }
    let ended = !state.state.is_active();
    let waits_for_lease = state.waits_for_lease;
    let approval = state.approval.clone();
    let taken = match services.publish_task_state(WORKERS_KIND, state).await {
        Ok(taken) => taken,
        Err(error) => {
            tracing::error!(%error, error.code = error.code(), task = %row.id, "could not store a worker's task state");
            return;
        }
    };
    let Some(row) = taken else {
        return;
    };
    let document = document_of(&row);
    if document.submission == Some(Submission::Pending) {
        shared
            .change_document(&row.id, |document| {
                document.submission = Some(Submission::Accepted);
            })
            .await;
    }
    if ended {
        shared.forget_lease(&row.id);
    }
    if let Some(approval) = &approval
        && !ended
    {
        answer_by_full_access(shared, worker, &row, approval).await;
    }
    if ended && document.profile_id.is_some() {
        revoke_ended_lease(shared, worker, &row.id).await;
    } else if waits_for_lease {
        lease_to(shared, worker, &row).await;
    }
}

/// Answers a worker's request at once when the task's parent conversation is
/// in full access and the worker takes such answers, as it says in its
/// `Hello` (ADR-0029, decision 5). Otherwise the request waits for the owner
/// in the task's card.
async fn answer_by_full_access(
    shared: &Shared,
    worker: &WorkerId,
    task: &StoredTask,
    approval: &ChildApproval,
) {
    let Some(request) = &approval.call.request else {
        return;
    };
    if !shared
        .services
        .parent_has_full_access(&task.conversation_id)
        .await
    {
        return;
    }
    let accepts = shared
        .workers_now()
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
    match shared.delegation.allow_by_full_access(worker, input).await {
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

/// Revokes the lease of a task that ended. The worker drops a task's lease
/// when the task ends, so this finds nothing to drop unless the worker missed
/// that (docs/features/workers/credential-leases.md).
async fn revoke_ended_lease(shared: &Shared, worker: &WorkerId, task: &TaskId) {
    if let Err(error) = shared.delegation.revoke_lease(worker, task).await {
        tracing::info!(%error, task = %task, worker = %worker, "could not revoke the lease of a task that ended");
    }
}

/// Delivers a task's lease once: the credential of the task's profile, which
/// the runtime lends from the master's credential store (ADR-0010; ADR-0030,
/// decision 13). A profile without a credential needs no lease. After a
/// failed delivery, the next state that waits for the lease tries again.
pub(crate) async fn lease_to(shared: &Shared, worker: &WorkerId, task: &StoredTask) {
    if !shared.claim_lease(&task.id) {
        return;
    }
    if !send_lease(shared, worker, task).await {
        shared.forget_lease(&task.id);
    }
}

/// Sends a task's lease to its worker; false when the worker did not get a
/// lease that the task needs.
async fn send_lease(shared: &Shared, worker: &WorkerId, task: &StoredTask) -> bool {
    let Some(profile) = document_of(task).profile_id else {
        return true;
    };
    let lent = match shared.lender.lend_credential(&profile).await {
        Ok(Some(lent)) => lent,
        Ok(None) => return true,
        Err(LendError::ProfileMissing) => {
            tracing::warn!(task = %task.id, "the profile of a worker task was deleted; its lease cannot be delivered");
            return false;
        }
        Err(error) => {
            tracing::warn!(%error, task = %task.id, "could not read the credential of a worker task");
            return false;
        }
    };
    match shared
        .delegation
        .deliver_lease(worker, &task.id, lent.binding, lent.credential)
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

/// Ends a worker task on the master, as when its worker does not know it or
/// was removed, and tells the run that follows it.
async fn end_remote_task(shared: &Shared, task: &StoredTask, state: TaskState) {
    if let Err(error) = shared
        .services
        .end_executor_task(WORKERS_KIND, task, state)
        .await
    {
        tracing::error!(%error, error.code = error.code(), task = %task.id, "could not end a worker task");
        return;
    }
    shared.forget_lease(&task.id);
}

/// Forgets a worker that was removed: its active tasks end, since it
/// canceled them when it was unpaired or can no longer report them, and the
/// deletions that waited for it are dropped (ADR-0031, decision 9).
async fn forget_worker_tasks(shared: &Shared, worker: &WorkerId) {
    let services = &shared.services;
    if let Ok(active) = services.active_executor_tasks(WORKERS_KIND).await {
        for task in active
            .iter()
            .filter(|task| worker_of(task) == Some(worker.as_str()))
        {
            end_remote_task(shared, task, TaskState::Interrupted).await;
        }
    }
    if let Err(error) = services
        .drop_executor_target_outbox(WORKERS_KIND, worker.as_str())
        .await
    {
        tracing::error!(%error, error.code = error.code(), worker = %worker, "could not drop the deletions that waited for a removed worker");
    }
    tracing::info!(worker = %worker, "stopped following a removed worker");
}
