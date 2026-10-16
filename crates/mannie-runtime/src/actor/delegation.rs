//! The side of a run that gives tasks to workers (ADR-0027; docs/features/
//! workers/delegation.md, submitting a task): resolving the worker, the part,
//! the rows, the submission and the lease, follow-ups on the same worker, and
//! the cancels of a stop.
//!
//! A worker task joins the run's lane as a child's task does. Its states come
//! from the runtime's follower of its worker, which publishes them on the
//! run's board while the run holds the task's [`RemoteFollow`].

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use mannie_store::{
    Durability, NewConversation, NewDelegatedTask, RuntimeStoreError, StoredChild, StoredTask,
    TaskEnding, TaskOrigin,
};
use mannie_tools::clean_label;
use mannie_types::ids::{TaskId, WorkerId};
use mannie_types::run::RunFailure;
use mannie_types::tasks::{
    AssignedTaskState, INSTRUCTION_PREVIEW_CHARS, TaskDocument, TaskRefusal, TaskState, TaskWorker,
};
use mannie_types::tools::ToolCallStatus;
use mannie_types::workers::{ProfileSnapshot, TaskAssignment, WorkerStatus, WorkerView};
use tokio::time::Instant;

use super::tasks::LaneTask;
use super::{Ending, RunActor, StepCall};
use crate::agents::WORKER_AGENT;
use crate::calls::Setup;
use crate::delegation::{Delegation, DelegationError, RemoteFollow, Submitting};
use crate::texts;

/// How long a run keeps trying to submit a task to a worker that cannot be
/// reached before it gives up on the task.
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(30);

/// The most characters of a worker's name as the runtime shows it.
const MAX_WORKER_NAME_CHARS: usize = 64;

/// A worker's name as the runtime shows it and the model names it: cleaned
/// of control characters and of the controls of bidirectional text, which
/// a worker could put in its name.
pub(crate) fn worker_label(view: &WorkerView) -> String {
    clean_label(&view.name, MAX_WORKER_NAME_CHARS).unwrap_or_default()
}

/// Why a worker cannot take a task now, if it cannot.
fn unavailable(view: &WorkerView, label: &str) -> Option<TaskRefusal> {
    let worker = label.to_owned();
    match view.status {
        WorkerStatus::Connected => None,
        WorkerStatus::Incompatible => Some(TaskRefusal::WorkerIncompatible { worker }),
        WorkerStatus::Connecting
        | WorkerStatus::Offline
        | WorkerStatus::KeyChanged
        | WorkerStatus::Unpaired => Some(TaskRefusal::WorkerOffline { worker }),
    }
}

/// Whether a host names this computer's loopback.
fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// The host of a worker's address, `host:port` or `[v6]:port`.
fn address_host(address: &str) -> &str {
    match address.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|character| character.is_ascii_digit()) => host,
        _ => address,
    }
}

/// Whether a profile whose base URL names this computer's loopback cannot be
/// reached from a worker: it names the worker's own host there, and the
/// credential must not go to it (docs/features/workers/credential-leases.md).
/// A worker whose own address is a loopback address runs on this computer,
/// as in tests.
fn unreachable_profile(setup: &Setup, view: &WorkerView) -> bool {
    let profile_loopback = setup
        .target
        .base_url
        .host_str()
        .is_some_and(is_loopback_host);
    profile_loopback && !is_loopback_host(address_host(&view.address))
}

/// The refusal of a call whose task no worker took.
fn refusal_of(
    error: &DelegationError,
    document: &TaskDocument,
    worker: &TaskWorker,
) -> TaskRefusal {
    match error {
        DelegationError::Refused { code } if code == "assign.still_running" => {
            TaskRefusal::StillRunning {
                number: document.number,
            }
        }
        DelegationError::Refused { .. } | DelegationError::Unreachable => {
            TaskRefusal::WorkerOffline {
                worker: worker.name.clone(),
            }
        }
    }
}

/// The start of an instruction, which the parent's part keeps for the owner.
fn instruction_preview(instruction: &str) -> String {
    instruction
        .chars()
        .take(INSTRUCTION_PREVIEW_CHARS)
        .collect()
}

/// What a delegation stores and submits, past its checks.
struct Delegated {
    document: TaskDocument,
    /// The task's conversation, for a new task.
    conversation: Option<NewConversation>,
    instruction: String,
    read_only: bool,
}

impl RunActor {
    /// The paired worker that a call names by its name, with the refusal of
    /// a worker that cannot take a task now. A model names a worker by the
    /// name the instructions showed it.
    fn worker_named(&self, name: &str) -> Result<(WorkerView, String), TaskRefusal> {
        let workers = self.runtime.workers_now();
        let found = workers.into_iter().find(|view| worker_label(view) == name);
        let Some(view) = found else {
            return Err(TaskRefusal::WorkerUnknown {
                worker: name.to_owned(),
            });
        };
        let label = worker_label(&view);
        match unavailable(&view, &label) {
            Some(refusal) => Err(refusal),
            None => Ok((view, label)),
        }
    }

    /// Runs a call of `delegate_task`: checks the budget, the worker and the
    /// profile, then gives the worker the task.
    #[expect(
        clippy::too_many_arguments,
        reason = "the arguments are the call's, as `start_task` takes its own"
    )]
    pub(super) async fn delegate(
        &mut self,
        setup: &Setup,
        call: StepCall,
        worker: &str,
        title: String,
        instruction: String,
        background: bool,
        read_only: bool,
    ) -> Result<(), Ending> {
        if self.task_budget_used(setup) {
            self.refuse_task(call, TaskRefusal::BudgetUsed);
            return Ok(());
        }
        let (view, label) = match self.worker_named(worker) {
            Ok(found) => found,
            Err(refusal) => {
                self.refuse_task(call, refusal);
                return Ok(());
            }
        };
        if unreachable_profile(setup, &view) {
            self.refuse_task(call, TaskRefusal::ProfileNotReachable { worker: label });
            return Ok(());
        }
        let number = self
            .runtime
            .inner
            .store
            .next_task_number(&self.conversation)
            .await?;
        let ids = &self.runtime.inner.ids;
        let task_worker = TaskWorker {
            id: view.id.clone(),
            name: label,
        };
        let document = TaskDocument {
            task_id: ids.task(),
            number,
            conversation_id: ids.conversation(),
            agent: WORKER_AGENT.to_owned(),
            title,
            background,
            follow_up: false,
            state: TaskState::Accepted,
            instruction: instruction_preview(&instruction),
            activity: None,
            tool_calls: 0,
            approval: None,
            report: None,
            failure: None,
            started_at: None,
            ended_at: None,
            worker: Some(task_worker.clone()),
            detached: false,
        };
        let conversation = NewConversation {
            id: document.conversation_id.clone(),
            title: document.title.clone(),
            profile_id: Some(self.profile.clone()),
            project_id: None,
            task: Some(TaskOrigin {
                parent_id: self.conversation.clone(),
                number,
                agent: WORKER_AGENT.to_owned(),
                worker: Some(task_worker),
            }),
        };
        let delegated = Delegated {
            document,
            conversation: Some(conversation),
            instruction,
            read_only,
        };
        self.submit_to_worker(call, delegated).await
    }

    /// Gives a worker a further task in the conversation of an earlier one,
    /// once that task ended. The worker that ran it runs it again.
    pub(super) async fn follow_up_on_worker(
        &mut self,
        setup: &Setup,
        call: StepCall,
        child: StoredChild,
        instruction: String,
        background: bool,
    ) -> Result<(), Ending> {
        let number = child.number;
        let still_running = child
            .latest
            .as_ref()
            .is_some_and(|task| task.state.is_active());
        if still_running {
            self.refuse_task(call, TaskRefusal::StillRunning { number });
            return Ok(());
        }
        let Some(worker) = child.worker else {
            self.refuse_task(call, TaskRefusal::UnknownTask { number });
            return Ok(());
        };
        let workers = self.runtime.workers_now();
        let Some(view) = workers.into_iter().find(|view| view.id == worker.id) else {
            self.refuse_task(
                call,
                TaskRefusal::WorkerUnknown {
                    worker: worker.name,
                },
            );
            return Ok(());
        };
        let label = worker_label(&view);
        if let Some(refusal) = unavailable(&view, &label) {
            self.refuse_task(call, refusal);
            return Ok(());
        }
        if unreachable_profile(setup, &view) {
            self.refuse_task(call, TaskRefusal::ProfileNotReachable { worker: label });
            return Ok(());
        }
        let document = TaskDocument {
            task_id: self.runtime.inner.ids.task(),
            number,
            conversation_id: child.conversation_id,
            agent: WORKER_AGENT.to_owned(),
            title: child.title,
            background,
            follow_up: true,
            state: TaskState::Accepted,
            instruction: instruction_preview(&instruction),
            activity: None,
            tool_calls: 0,
            approval: None,
            report: None,
            failure: None,
            started_at: None,
            ended_at: None,
            worker: Some(TaskWorker {
                id: view.id.clone(),
                name: label,
            }),
            detached: false,
        };
        let delegated = Delegated {
            document,
            conversation: None,
            instruction,
            // A follow-up keeps what the worker may do as the instruction
            // asks anew; it reads only when the model narrows it again.
            read_only: false,
        };
        self.submit_to_worker(call, delegated).await
    }

    /// Gives a worker its task (docs/features/workers/delegation.md,
    /// submitting a task): the part records the task first; then the task's
    /// rows are stored, with the submission pending; then the task is
    /// submitted, and its lease delivered. Mannie's end after the part and
    /// before the rows leaves the part "not started"; after the rows, the
    /// part lets the task go, and the worker's follower learns what became
    /// of it.
    async fn submit_to_worker(
        &mut self,
        mut call: StepCall,
        delegated: Delegated,
    ) -> Result<(), Ending> {
        let Some(delegation) = self.runtime.delegation().map(Arc::clone) else {
            let worker = delegated
                .document
                .worker
                .map(|worker| worker.name)
                .unwrap_or_default();
            self.refuse_task(call, TaskRefusal::WorkerUnknown { worker });
            return Ok(());
        };
        let Delegated {
            document,
            conversation,
            instruction,
            read_only,
        } = delegated;
        let Some(worker) = document.worker.clone() else {
            return Err(Ending::Failed(RunFailure::Internal));
        };
        let snapshot = self.profile_snapshot().await?;
        self.tasks.assigned += 1;
        self.record_in_part(&mut call, &document).await?;
        let Some(row) = self
            .record_rows(&mut call, &document, conversation, &worker)
            .await?
        else {
            self.refuse_task(
                call,
                TaskRefusal::StillRunning {
                    number: document.number,
                },
            );
            return Ok(());
        };
        // The run holds the task's states before the worker can publish any.
        let follow = RemoteFollow::new(&self.runtime, &row.id, &self.tasks.board);
        let assignment = TaskAssignment {
            task_id: document.task_id.clone(),
            conversation_id: document.conversation_id.clone(),
            follow_up: document.follow_up,
            title: document.title.clone(),
            instruction,
            read_only,
            profile: snapshot,
        };
        let submitted = {
            let _submitting = Submitting::new(&self.runtime, &row.id);
            self.submit(&delegation, &worker.id, assignment).await
        };
        let state = match submitted {
            Ok(Ok(state)) => state,
            Ok(Err(error)) => {
                tracing::info!(run = %self.run, task = %row.id, worker = %worker.id, %error, "a worker did not take a task");
                self.end_unsubmitted(&row.id, TaskState::Interrupted).await;
                let refusal = refusal_of(&error, &document, &worker);
                if let Some(section) = &mut call.task {
                    section.task = None;
                }
                self.refuse_task(call, refusal);
                return Ok(());
            }
            Err(ending) => {
                // A stop during the submission cancels the task, which the
                // worker may have stored; the cancel waits for the worker.
                if matches!(ending, Ending::Stopped) {
                    self.queue_cancel(&row.id).await;
                }
                return Err(ending);
            }
        };
        let active = state.state.is_active();
        self.runtime
            .receive_state(&delegation, &worker.id, state)
            .await;
        // The worker's run asks for the lease once it starts; sending it now
        // saves that round trip, and the follower does not send it again.
        if active {
            self.runtime.lease_to(&delegation, &worker.id, &row).await;
        }
        tracing::info!(
            run = %self.run,
            task = %row.id,
            number = document.number,
            worker = %worker.id,
            follow_up = document.follow_up,
            background = document.background,
            read_only,
            "a worker got a task"
        );
        self.add_remote_task(call, document, follow);
        self.flush(Durability::Full).await
    }

    /// Records a task in its call's part, durably, before anything else
    /// stores it (step 1 of a submission).
    async fn record_in_part(
        &mut self,
        call: &mut StepCall,
        document: &TaskDocument,
    ) -> Result<(), Ending> {
        call.document.status = ToolCallStatus::Running;
        call.document.started_at = Some(self.now());
        let mut section = call.task.take().unwrap_or_default();
        section.task = Some(document.clone());
        call.task = Some(section);
        self.pending.push(call.update(Vec::new()));
        self.flush(Durability::Full).await
    }

    /// Stores a task's rows, and its conversation's for a new task, with the
    /// submission pending (step 2 of a submission). `None` when the task's
    /// conversation still works on an earlier task: nothing was stored, and
    /// the part no longer records the task.
    async fn record_rows(
        &mut self,
        call: &mut StepCall,
        document: &TaskDocument,
        conversation: Option<NewConversation>,
        worker: &TaskWorker,
    ) -> Result<Option<StoredTask>, Ending> {
        let recorded = self
            .runtime
            .inner
            .store
            .record_delegated_task(NewDelegatedTask {
                id: document.task_id.clone(),
                conversation_id: document.conversation_id.clone(),
                conversation,
                part_id: call.part_id.clone(),
                worker_id: worker.id.clone(),
                profile_id: self.profile.clone(),
                follow_up: document.follow_up,
                background: document.background,
            })
            .await;
        match recorded {
            Ok(row) => Ok(Some(row)),
            Err(RuntimeStoreError::TaskActive) => {
                self.tasks.assigned -= 1;
                if let Some(section) = &mut call.task {
                    section.task = None;
                }
                Ok(None)
            }
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), run = %self.run, "could not store a task for a worker");
                Err(Ending::Failed(RunFailure::Internal))
            }
        }
    }

    /// Submits a task, trying again while the worker cannot be reached, up
    /// to the submit timeout. A worker stores a task once, so trying again
    /// after an answer that was lost finds the same task. Stop and Shutdown
    /// end the tries.
    async fn submit(
        &mut self,
        delegation: &Arc<dyn Delegation>,
        worker: &WorkerId,
        assignment: TaskAssignment,
    ) -> Result<Result<AssignedTaskState, DelegationError>, Ending> {
        let deadline = Instant::now() + SUBMIT_TIMEOUT;
        loop {
            let submitting = delegation.submit(worker, assignment.clone());
            match self.wait(submitting).await? {
                Err(DelegationError::Unreachable) if Instant::now() < deadline => {
                    tracing::info!(run = %self.run, worker = %worker, "a worker could not be reached to submit a task; trying again");
                    let mut workers = delegation.workers();
                    let connected = async move {
                        // The borrow that `wait_for` returns is dropped at
                        // once, so it never holds up the list's changes.
                        workers
                            .wait_for(|views| {
                                views.iter().any(|view| {
                                    view.id == *worker && view.status == WorkerStatus::Connected
                                })
                            })
                            .await
                            .is_ok()
                    };
                    let timed = tokio::time::timeout_at(deadline, connected);
                    let _waited = self.wait(timed).await?;
                }
                answer => return Ok(answer),
            }
        }
    }

    /// Ends the row of a task that no worker took.
    async fn end_unsubmitted(&self, task: &TaskId, state: TaskState) {
        let ending = TaskEnding {
            state,
            report: None,
            failure: None,
        };
        if let Err(error) = self
            .runtime
            .inner
            .store
            .apply_remote_task_state(task, 0, state, Some(ending))
            .await
        {
            tracing::error!(%error, error.code = error.code(), task = %task, "could not end a task that no worker took");
        }
        self.runtime.inner.followers.forget_lease(task);
    }

    /// Stores a cancel for a task on a worker and wakes its follower, which
    /// sends it while the worker is connected and again after each
    /// connection until the task ends.
    async fn queue_cancel(&self, task: &TaskId) {
        match self.runtime.inner.store.request_task_cancel(task).await {
            Ok(_active) => self.runtime.inner.followers.nudge(),
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), task = %task, "could not store the cancel of a task on a worker");
            }
        }
    }

    /// Cancels the run's tasks that still run on workers, because the run
    /// stops: each cancel is stored, so it reaches the worker even if the
    /// worker cannot be reached now, and sent at once.
    pub(super) async fn cancel_remote_tasks(&mut self) {
        let remote: Vec<_> = self
            .tasks
            .tasks
            .iter()
            .filter(|task| task.ending.is_none() && task.mailbox.is_none())
            .map(|task| task.id.clone())
            .collect();
        for task in &remote {
            self.queue_cancel(task).await;
        }
        if !remote.is_empty() {
            tracing::info!(run = %self.run, tasks = remote.len(), "canceling the run's tasks on workers");
        }
    }

    /// The snapshot of the run's profile that a worker gets with a task,
    /// without its credential.
    async fn profile_snapshot(&self) -> Result<ProfileSnapshot, Ending> {
        let profile = self
            .runtime
            .inner
            .store
            .profile(&self.profile)
            .await
            .map_err(|error| {
                tracing::error!(%error, error.code = error.code(), "could not read the profile of a run");
                Ending::Failed(RunFailure::Internal)
            })?
            .ok_or(Ending::Failed(RunFailure::ProfileMissing))?;
        Ok(ProfileSnapshot {
            id: profile.id.clone(),
            config_revision: profile.config_revision,
            interface: profile.interface,
            base_url: profile.base_url.clone(),
            model: profile.model.clone(),
            context_window: profile.context_window,
            output_limit: profile.output_limit,
            reasoning_effort: profile.reasoning_effort,
            server: profile.server,
            use_tools: profile.use_tools,
            has_credential: profile.credential_ref.is_some(),
        })
    }

    /// Keeps a task on a worker in the lane until it ends. A task in the
    /// background gets its result at once.
    fn add_remote_task(
        &mut self,
        mut call: StepCall,
        document: TaskDocument,
        follow: RemoteFollow,
    ) {
        let mut section = call.task.take().unwrap_or_default();
        section.task = None;
        let mut task = LaneTask {
            id: document.task_id.clone(),
            part_id: call.part_id,
            call: call.document,
            section,
            document,
            mailbox: None,
            follow: Some(follow),
            ending: None,
            read: false,
            noted: false,
        };
        if task.document.background {
            let worker = task
                .document
                .worker
                .as_ref()
                .map(|worker| worker.name.as_str())
                .unwrap_or_default();
            // The model reads this result at once; the part stays open until
            // the task ends.
            task.section.output = Some(texts::worker_task_started_in_background(
                task.document.number,
                worker,
            ));
            self.pending.push(task.update(Vec::new()));
        }
        self.tasks.tasks.push(task);
    }
}

#[cfg(test)]
mod tests {
    use super::{address_host, is_loopback_host};

    #[test]
    fn loopback_hosts_are_recognized_in_urls_and_addresses() {
        for host in [
            "localhost",
            "LOCALHOST",
            "127.0.0.1",
            "127.8.9.1",
            "[::1]",
            "::1",
        ] {
            assert!(is_loopback_host(host), "{host}");
        }
        for host in ["build-01.lan", "10.0.0.5", "[fd00::1]", "example.com"] {
            assert!(!is_loopback_host(host), "{host}");
        }
        assert_eq!(address_host("build-01.lan:7445"), "build-01.lan");
        assert_eq!(address_host("[::1]:7445"), "[::1]");
        assert_eq!(address_host("build-01.lan"), "build-01.lan");
    }
}
