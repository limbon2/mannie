//! The Workers plugin's executor (ADR-0031, decisions 4 and 5): the tool
//! `delegate_task` and the list of workers, the checks of a call, the
//! submission of a task to its worker, cancels, and the reads of a worker
//! task's conversation through its worker. The followers in `follow.rs`
//! publish each task's states to the runtime.

use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use mannie_provider::ToolDefinition;
use mannie_runtime::{
    CredentialLender, ExecutorRefusal, ExecutorSection, ExecutorServices, FollowUpPlan, PlanCall,
    PlanError, SubmitError, TaskExecutor, TaskPlan, TaskSubmission,
};
use mannie_tools::{clean_label, with_activity};
use mannie_types::chat::{ChatError, CheckpointRef, EarlierTurns, LoadEarlierTurns};
use mannie_types::clock::{Clock, UtcMillis};
use mannie_types::ids::{ConversationId, ProfileId, TaskId, WorkerId};
use mannie_types::tasks::{
    AssignedTaskState, MAX_INSTRUCTION_BYTES, MAX_TITLE_CHARS, ProfileSnapshot, TaskAssignment,
    TaskExecutorRef,
};
use mannie_types::tools::{DecideApproval, ReadToolOutput, ToolOutputPage, ToolsError};
use mannie_types::transcript::{Checkpoint, ConversationUpdate};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;
use tokio::time::Instant;

use crate::delegation::{Delegation, DelegationError};
use crate::texts::{self, THE_WORKER, worker_label};
use crate::view::{WorkerStatus, WorkerView};

/// The executor's kind: the plugin's ID (ADR-0030, decision 3).
pub const WORKERS_KIND: &str = "workers";

/// The agent that the master records for a task that a worker runs.
pub const WORKER_AGENT: &str = "worker";

/// The executor's tool (ADR-0027, decision 2).
pub const DELEGATE_TASK: &str = "delegate_task";

/// Where the list of workers goes among the instruction sections, as in
/// 0.2.0: after the task tools' guidance.
const WORKERS_SECTION_POSITION: u32 = 310;

/// How long a submission keeps trying a worker that cannot be reached before
/// it gives up on the task.
const SUBMIT_TIMEOUT: Duration = Duration::from_secs(30);

/// The refusal codes of the executor, which the UI shows the plugin's
/// messages for (ADR-0031, decision 6). They are the names of 0.2.0's
/// refusals.
pub mod codes {
    /// The worker is not connected.
    pub const WORKER_OFFLINE: &str = "worker_offline";
    /// No paired worker has the name.
    pub const WORKER_UNKNOWN: &str = "worker_unknown";
    /// The worker runs a version that this one does not work with.
    pub const WORKER_INCOMPATIBLE: &str = "worker_incompatible";
    /// The profile uses an address on this computer.
    pub const PROFILE_NOT_REACHABLE: &str = "profile_not_reachable";
    /// The task's conversation still works on an earlier task on the worker.
    pub const STILL_RUNNING: &str = "still_running";
}

/// Whether the worker has stored a task, as the executor's document records
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Submission {
    /// The submission was sent, and the worker has not answered yet.
    Pending,
    /// The worker stored the task.
    Accepted,
}

/// What the executor keeps of a worker task on the task's row in the core
/// (ADR-0031, decision 8): the profile whose credential it leases, whether
/// the worker stored the task, and a cancel that waits for the worker. M5's
/// migration wrote it from 0.2.0's columns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkerTaskDocument {
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) profile_id: Option<ProfileId>,
    #[serde(default)]
    pub(crate) submission: Option<Submission>,
    #[serde(default)]
    pub(crate) cancel_requested_at: Option<UtcMillis>,
}

/// The version of the documents that this version writes.
const DOCUMENT_VERSION: u32 = 1;

impl WorkerTaskDocument {
    /// The document of a stored row, or the default for one that does not
    /// parse, which leases nothing and waits for no cancel.
    pub(crate) fn of(document: Option<&str>, task: &TaskId) -> Self {
        let Some(text) = document else {
            return Self::default();
        };
        serde_json::from_str(text).unwrap_or_else(|error| {
            tracing::warn!(%error, task = %task, "the document of a worker task is not valid");
            Self::default()
        })
    }

    fn to_json(&self) -> Option<String> {
        serde_json::to_string(self).ok()
    }
}

/// The arguments of `delegate_task`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DelegateArguments {
    worker: String,
    title: String,
    instruction: String,
    background: bool,
    read_only: bool,
}

/// What the executor's plan adds to a submission.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitOptions {
    read_only: bool,
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every critical section only inserts or removes entries, so the data
    // stays consistent even if a holder panicked.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The executor's state, which its followers share.
#[derive(Debug)]
pub(crate) struct Shared {
    pub(crate) services: ExecutorServices,
    pub(crate) lender: CredentialLender,
    pub(crate) delegation: Arc<dyn Delegation>,
    pub(crate) clock: Arc<dyn Clock>,
    /// The tasks whose submission a run is making now, which a follower's
    /// comparison with the worker leaves alone.
    pub(crate) submitting: Mutex<HashSet<TaskId>>,
    /// The tasks whose lease was delivered or is being delivered: the run
    /// that submits a task and the follower of its worker both offer the
    /// lease, and the worker gets it once. A failed delivery and the task's
    /// end take the task out again.
    pub(crate) leases: Mutex<HashSet<TaskId>>,
    /// Counts the cancels that were queued, so a follower sends them while
    /// its worker is connected.
    pub(crate) queued: watch::Sender<u64>,
    /// Tells the followers and the submissions that the plugin stops.
    pub(crate) stopping: watch::Sender<bool>,
    /// Serializes the changes to the executor's documents, which are read,
    /// changed and written back.
    pub(crate) documents: tokio::sync::Mutex<()>,
}

impl Shared {
    /// Takes on the delivery of a task's lease; false when the lease was
    /// delivered or is being delivered.
    pub(crate) fn claim_lease(&self, task: &TaskId) -> bool {
        lock(&self.leases).insert(task.clone())
    }

    /// Lets the next state that waits for a task's lease deliver it again.
    pub(crate) fn forget_lease(&self, task: &TaskId) {
        lock(&self.leases).remove(task);
    }

    pub(crate) fn is_submitting(&self, task: &TaskId) -> bool {
        lock(&self.submitting).contains(task)
    }

    /// Wakes the followers to send the queued cancels.
    pub(crate) fn nudge(&self) {
        self.queued
            .send_modify(|count| *count = count.wrapping_add(1));
    }

    /// The paired workers as they are now.
    pub(crate) fn workers_now(&self) -> Vec<WorkerView> {
        self.delegation.workers().borrow().clone()
    }

    /// Changes the executor's document of a task. Returns the document as it
    /// is after the change, or `None` for a task that is not a worker task.
    pub(crate) async fn change_document(
        &self,
        task: &TaskId,
        change: impl FnOnce(&mut WorkerTaskDocument),
    ) -> Option<WorkerTaskDocument> {
        let _changing = self.documents.lock().await;
        let row = match self.services.executor_task(task).await {
            Ok(Some(row)) => row,
            Ok(None) => return None,
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), task = %task, "could not read a worker task");
                return None;
            }
        };
        let execution = row
            .executor
            .filter(|execution| execution.kind == WORKERS_KIND)?;
        let mut document = WorkerTaskDocument::of(execution.document.as_deref(), task);
        change(&mut document);
        document.version = DOCUMENT_VERSION;
        match self
            .services
            .set_executor_document(WORKERS_KIND, task, document.to_json())
            .await
        {
            Ok(true) => Some(document),
            Ok(false) => None,
            Err(error) => {
                tracing::error!(%error, error.code = error.code(), task = %task, "could not store the document of a worker task");
                None
            }
        }
    }
}

/// The executor of the Workers plugin. Cloning is cheap; clones share
/// everything.
///
/// The runtime holds the executor while the plugin is on, and the executor
/// holds the runtime's services for it; [`WorkersExecutor::stop`] and the
/// runtime's removal of the plugin end that cycle when the plugin stops.
#[derive(Debug, Clone)]
pub struct WorkersExecutor {
    pub(crate) shared: Arc<Shared>,
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
fn unreachable_profile(profile: &ProfileSnapshot, view: &WorkerView) -> bool {
    let profile_loopback = mannie_provider::Url::parse(&profile.base_url)
        .ok()
        .and_then(|url| url.host_str().map(is_loopback_host))
        .unwrap_or(false);
    profile_loopback && !is_loopback_host(address_host(&view.address))
}

fn refusal(code: &str, worker: &str, text: String) -> ExecutorRefusal {
    ExecutorRefusal {
        code: code.to_owned(),
        target: Some(worker.to_owned()),
        text,
    }
}

/// Why a worker cannot take a task now, if it cannot.
fn unavailable(view: &WorkerView, label: &str) -> Option<ExecutorRefusal> {
    match view.status {
        WorkerStatus::Connected => None,
        WorkerStatus::Incompatible => Some(refusal(
            codes::WORKER_INCOMPATIBLE,
            label,
            texts::worker_incompatible(label),
        )),
        WorkerStatus::Connecting
        | WorkerStatus::Offline
        | WorkerStatus::KeyChanged
        | WorkerStatus::Unpaired => Some(refusal(
            codes::WORKER_OFFLINE,
            label,
            texts::worker_offline(label),
        )),
    }
}

/// The target of a worker task.
fn target_of(view: &WorkerView, label: String) -> TaskExecutorRef {
    TaskExecutorRef {
        kind: WORKERS_KIND.to_owned(),
        target_id: view.id.as_str().to_owned(),
        target_name: label,
        noun: THE_WORKER.to_owned(),
    }
}

/// Checks an instruction as the core checks those of its task tools: not
/// blank, and not longer than the limit.
fn check_instruction(instruction: &str) -> Result<(), String> {
    if instruction.trim().is_empty() {
        return Err("the instruction is empty".to_owned());
    }
    if instruction.len() > MAX_INSTRUCTION_BYTES {
        return Err(format!(
            "the instruction is longer than {MAX_INSTRUCTION_BYTES} bytes"
        ));
    }
    Ok(())
}

/// Reads the arguments of a call of `delegate_task`. The error says what is
/// wrong, for the model.
fn parse_arguments(arguments: &str) -> Result<(DelegateArguments, String), String> {
    let trimmed = arguments.trim();
    let text = if trimmed.is_empty() || trimmed == "null" {
        "{}"
    } else {
        arguments
    };
    let input: DelegateArguments = serde_json::from_str(text).map_err(|error| error.to_string())?;
    let title = clean_label(&input.title, MAX_TITLE_CHARS)
        .ok_or_else(|| "the title is empty".to_owned())?;
    if input.worker.trim().is_empty() {
        return Err("the worker is empty".to_owned());
    }
    check_instruction(&input.instruction)?;
    Ok((input, title))
}

/// The parameters of `delegate_task`, strict like the core's task tools: every
/// property is required and nothing else is allowed. `workers` are the names
/// of the paired workers.
fn delegate_parameters(workers: &[String]) -> Value {
    json!({
        "additionalProperties": false,
        "properties": {
            "background": {
                "description": texts::DELEGATE_BACKGROUND_ARGUMENT,
                "type": "boolean",
            },
            "instruction": {
                "description": texts::DELEGATE_INSTRUCTION_ARGUMENT,
                "type": "string",
            },
            "read_only": {
                "description": texts::DELEGATE_READ_ONLY_ARGUMENT,
                "type": "boolean",
            },
            "title": {
                "description": mannie_runtime::texts::TASK_TITLE_ARGUMENT,
                "type": "string",
            },
            "worker": {
                "description": texts::DELEGATE_WORKER_ARGUMENT,
                "enum": workers,
                "type": "string",
            },
        },
        "required": ["background", "instruction", "read_only", "title", "worker"],
        "type": "object",
    })
}

impl WorkersExecutor {
    /// An executor that reaches the workers through `delegation`. It offers
    /// its tool at once; [`WorkersExecutor::start`] starts following the
    /// workers' tasks.
    pub fn new(
        services: ExecutorServices,
        lender: CredentialLender,
        delegation: Arc<dyn Delegation>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                services,
                lender,
                delegation,
                clock,
                submitting: Mutex::default(),
                leases: Mutex::default(),
                queued: watch::Sender::new(0),
                stopping: watch::Sender::new(false),
                documents: tokio::sync::Mutex::new(()),
            }),
        }
    }

    /// Ends the followers, and the submissions that wait for a worker, which
    /// the runs let go, as the plugin stops (ADR-0031, decision 10). Nothing
    /// is cancelled.
    pub fn stop(&self) {
        self.shared.stopping.send_replace(true);
    }

    /// The paired worker that a call names by its name, with the refusal of
    /// a worker that cannot take a task now. A model names a worker by the
    /// name the instructions showed it.
    fn worker_named(&self, name: &str) -> Result<(WorkerView, String), ExecutorRefusal> {
        let found = self
            .shared
            .workers_now()
            .into_iter()
            .find(|view| worker_label(view) == name);
        let Some(view) = found else {
            return Err(refusal(
                codes::WORKER_UNKNOWN,
                name,
                texts::no_such_worker(name),
            ));
        };
        let label = worker_label(&view);
        match unavailable(&view, &label) {
            Some(refused) => Err(refused),
            None => Ok((view, label)),
        }
    }

    /// Submits a task, trying again while the worker cannot be reached, up
    /// to the submit timeout. A worker stores a task once, so trying again
    /// after an answer that was lost finds the same task. The plugin's stop
    /// ends the tries.
    async fn submit_until_answered(
        shared: &Shared,
        worker: &WorkerId,
        assignment: TaskAssignment,
    ) -> Result<Result<AssignedTaskState, DelegationError>, SubmitError> {
        let mut stopping = shared.stopping.subscribe();
        let deadline = Instant::now() + SUBMIT_TIMEOUT;
        loop {
            if *stopping.borrow_and_update() {
                return Err(SubmitError::Stopped);
            }
            let submitted = tokio::select! {
                submitted = shared.delegation.submit(worker, assignment.clone()) => submitted,
                _stopping = stopping.changed() => return Err(SubmitError::Stopped),
            };
            match submitted {
                Err(DelegationError::Unreachable) if Instant::now() < deadline => {
                    tracing::info!(worker = %worker, task = %assignment.task_id, "a worker could not be reached to submit a task; trying again");
                    let mut workers = shared.delegation.workers();
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
                    tokio::select! {
                        _waited = tokio::time::timeout_at(deadline, connected) => {}
                        _stopping = stopping.changed() => return Err(SubmitError::Stopped),
                    }
                }
                answer => return Ok(answer),
            }
        }
    }
}

impl TaskExecutor for WorkersExecutor {
    fn kind(&self) -> &'static str {
        WORKERS_KIND
    }

    fn agent(&self) -> &'static str {
        WORKER_AGENT
    }

    /// `delegate_task`, offered while a worker is paired, so Mannie without
    /// workers offers what it offered before (ADR-0027, decision 2).
    fn tool(&self) -> Option<ToolDefinition> {
        let names: Vec<String> = self.shared.workers_now().iter().map(worker_label).collect();
        if names.is_empty() {
            return None;
        }
        Some(ToolDefinition {
            name: DELEGATE_TASK.to_owned(),
            description: texts::DELEGATE_TASK.to_owned(),
            parameters: with_activity(&delegate_parameters(&names)),
        })
    }

    fn section(&self) -> Option<ExecutorSection> {
        let workers = self.shared.workers_now();
        (!workers.is_empty()).then(|| ExecutorSection {
            position: WORKERS_SECTION_POSITION,
            text: texts::workers(&workers),
        })
    }

    fn outlives_reply(&self) -> bool {
        true
    }

    fn plan(&self, call: PlanCall<'_>) -> Result<TaskPlan, PlanError> {
        let (arguments, title) = parse_arguments(call.arguments)
            .map_err(|detail| PlanError::InvalidArguments { detail })?;
        let (view, label) = self
            .worker_named(&arguments.worker)
            .map_err(PlanError::Refused)?;
        if unreachable_profile(call.profile, &view) {
            return Err(PlanError::Refused(refusal(
                codes::PROFILE_NOT_REACHABLE,
                &label,
                texts::profile_not_reachable(&label),
            )));
        }
        let options = SubmitOptions {
            read_only: arguments.read_only,
        };
        Ok(TaskPlan {
            target: target_of(&view, label),
            title,
            instruction: arguments.instruction,
            background: arguments.background,
            options: serde_json::to_value(options).unwrap_or(Value::Null),
        })
    }

    fn plan_follow_up(
        &self,
        earlier: &TaskExecutorRef,
        profile: &ProfileSnapshot,
    ) -> Result<FollowUpPlan, ExecutorRefusal> {
        let found = self
            .shared
            .workers_now()
            .into_iter()
            .find(|view| view.id.as_str() == earlier.target_id);
        let Some(view) = found else {
            return Err(refusal(
                codes::WORKER_UNKNOWN,
                &earlier.target_name,
                texts::no_such_worker(&earlier.target_name),
            ));
        };
        let label = worker_label(&view);
        if let Some(refused) = unavailable(&view, &label) {
            return Err(refused);
        }
        if unreachable_profile(profile, &view) {
            return Err(refusal(
                codes::PROFILE_NOT_REACHABLE,
                &label,
                texts::profile_not_reachable(&label),
            ));
        }
        // A follow-up keeps what the worker may do as the instruction asks
        // anew; it reads only when the model narrows it again.
        let options = SubmitOptions { read_only: false };
        Ok(FollowUpPlan {
            target: target_of(&view, label),
            options: serde_json::to_value(options).unwrap_or(Value::Null),
        })
    }

    fn document(&self, submission: &TaskSubmission) -> Option<String> {
        WorkerTaskDocument {
            version: DOCUMENT_VERSION,
            profile_id: Some(submission.profile.id.clone()),
            submission: Some(Submission::Pending),
            cancel_requested_at: None,
        }
        .to_json()
    }

    /// Submits a task to its worker (docs/features/workers/delegation.md,
    /// submitting a task), and delivers its lease once the worker took it:
    /// the worker's run asks for the lease once it starts, and sending it
    /// now saves that round trip.
    fn submit(
        &self,
        submission: TaskSubmission,
    ) -> BoxFuture<'static, Result<AssignedTaskState, SubmitError>> {
        let shared = Arc::clone(&self.shared);
        async move {
            let label = submission.target.target_name.clone();
            let Ok(worker) = WorkerId::parse(&submission.target.target_id) else {
                return Err(SubmitError::Refused(refusal(
                    codes::WORKER_UNKNOWN,
                    &label,
                    texts::no_such_worker(&label),
                )));
            };
            let options: SubmitOptions =
                serde_json::from_value(submission.options.clone()).unwrap_or_default();
            let task = submission.task_id.clone();
            let assignment = TaskAssignment {
                task_id: task.clone(),
                conversation_id: submission.conversation_id.clone(),
                follow_up: submission.follow_up,
                title: submission.title.clone(),
                instruction: submission.instruction.clone(),
                read_only: options.read_only,
                profile: submission.profile.clone(),
            };
            lock(&shared.submitting).insert(task.clone());
            let submitted = Self::submit_until_answered(&shared, &worker, assignment).await;
            lock(&shared.submitting).remove(&task);
            let state = match submitted? {
                Ok(state) => state,
                Err(DelegationError::Refused { code }) if code == "assign.still_running" => {
                    return Err(SubmitError::Refused(refusal(
                        codes::STILL_RUNNING,
                        &label,
                        mannie_runtime::texts::task_still_running(submission.number),
                    )));
                }
                Err(error) => {
                    tracing::info!(task = %task, worker = %worker, %error, "a worker did not take a task");
                    return Err(SubmitError::Refused(refusal(
                        codes::WORKER_OFFLINE,
                        &label,
                        texts::worker_offline(&label),
                    )));
                }
            };
            shared
                .change_document(&task, |document| {
                    document.submission = Some(Submission::Accepted);
                })
                .await;
            if state.state.is_active()
                && let Ok(Some(row)) = shared.services.executor_task(&task).await
            {
                crate::follow::lease_to(&shared, &worker, &row).await;
            }
            tracing::info!(
                task = %task,
                number = submission.number,
                worker = %worker,
                follow_up = submission.follow_up,
                read_only = options.read_only,
                "a worker got a task"
            );
            Ok(state)
        }
        .boxed()
    }

    /// Stores a cancel for a task and wakes its worker's follower, which
    /// sends it while the worker is connected and again after each
    /// connection until the task ends.
    fn cancel(&self, task: &TaskId) -> BoxFuture<'static, ()> {
        let shared = Arc::clone(&self.shared);
        let task = task.clone();
        async move {
            let now = shared.clock.now();
            let stored = shared
                .change_document(&task, |document| {
                    document.cancel_requested_at.get_or_insert(now);
                })
                .await;
            if stored.is_some() {
                shared.nudge();
                tracing::info!(task = %task, "a cancel of a worker task waits for its worker");
            }
        }
        .boxed()
    }

    fn subscribe(
        &self,
        target: &TaskExecutorRef,
        conversation: &ConversationId,
        after_seq: Option<u32>,
    ) -> BoxFuture<'static, Result<BoxStream<'static, ConversationUpdate>, ChatError>> {
        let label = target.target_name.clone();
        let Ok(worker) = WorkerId::parse(&target.target_id) else {
            return futures_util::future::ready(Err(chat_unreachable(&label))).boxed();
        };
        let subscribing = self
            .shared
            .delegation
            .subscribe(&worker, conversation, after_seq);
        async move { subscribing.await.map_err(|error| chat_error(error, &label)) }.boxed()
    }

    fn earlier_turns(
        &self,
        target: &TaskExecutorRef,
        input: LoadEarlierTurns,
    ) -> BoxFuture<'static, Result<EarlierTurns, ChatError>> {
        let label = target.target_name.clone();
        let Ok(worker) = WorkerId::parse(&target.target_id) else {
            return futures_util::future::ready(Err(chat_unreachable(&label))).boxed();
        };
        let reading = self.shared.delegation.earlier_turns(&worker, input);
        async move { reading.await.map_err(|error| chat_error(error, &label)) }.boxed()
    }

    fn checkpoint(
        &self,
        target: &TaskExecutorRef,
        input: CheckpointRef,
    ) -> BoxFuture<'static, Result<Checkpoint, ChatError>> {
        let label = target.target_name.clone();
        let Ok(worker) = WorkerId::parse(&target.target_id) else {
            return futures_util::future::ready(Err(chat_unreachable(&label))).boxed();
        };
        let reading = self.shared.delegation.checkpoint(&worker, input);
        async move { reading.await.map_err(|error| chat_error(error, &label)) }.boxed()
    }

    fn tool_output(
        &self,
        target: &TaskExecutorRef,
        input: ReadToolOutput,
    ) -> BoxFuture<'static, Result<ToolOutputPage, ToolsError>> {
        let label = target.target_name.clone();
        let Ok(worker) = WorkerId::parse(&target.target_id) else {
            return futures_util::future::ready(Err(tools_unreachable(&label))).boxed();
        };
        let reading = self.shared.delegation.tool_output(&worker, input);
        async move { reading.await.map_err(|error| tools_error(error, &label)) }.boxed()
    }

    fn decide_approval(
        &self,
        target: &TaskExecutorRef,
        input: DecideApproval,
    ) -> BoxFuture<'static, Result<(), ToolsError>> {
        let label = target.target_name.clone();
        let Ok(worker) = WorkerId::parse(&target.target_id) else {
            return futures_util::future::ready(Err(tools_unreachable(&label))).boxed();
        };
        let deciding = self.shared.delegation.decide_approval(&worker, input);
        async move { deciding.await.map_err(|error| tools_error(error, &label)) }.boxed()
    }
}

/// The error of a read of a worker task's conversation whose worker cannot
/// be reached (ADR-0031, decision 6).
fn chat_unreachable(worker: &str) -> ChatError {
    ChatError::TaskTargetUnreachable {
        kind: WORKERS_KIND.to_owned(),
        target: worker.to_owned(),
    }
}

fn tools_unreachable(worker: &str) -> ToolsError {
    ToolsError::TaskTargetUnreachable {
        kind: WORKERS_KIND.to_owned(),
        target: worker.to_owned(),
    }
}

/// A worker's error of a read, with the link's "unreachable" named as the
/// executor's target.
fn chat_error(error: ChatError, worker: &str) -> ChatError {
    match error {
        ChatError::WorkerUnreachable => chat_unreachable(worker),
        other => other,
    }
}

fn tools_error(error: ToolsError, worker: &str) -> ToolsError {
    match error {
        ToolsError::WorkerUnreachable => tools_unreachable(worker),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::{address_host, is_loopback_host, parse_arguments};

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

    #[test]
    fn a_call_names_a_worker_a_title_and_an_instruction() {
        let valid = serde_json::json!({
            "worker": "build-01",
            "title": " Build\nit ",
            "instruction": "Build the backend.",
            "background": false,
            "read_only": true,
        })
        .to_string();
        let (arguments, title) = parse_arguments(&valid).unwrap();
        assert_eq!(title, "Build it");
        assert!(arguments.read_only);

        for (refused, why) in [
            (
                serde_json::json!({ "worker": "", "title": "T", "instruction": "I", "background": false, "read_only": false }),
                "worker",
            ),
            (
                serde_json::json!({ "worker": "w", "title": "  ", "instruction": "I", "background": false, "read_only": false }),
                "title",
            ),
            (
                serde_json::json!({ "worker": "w", "title": "T", "instruction": " ", "background": false, "read_only": false }),
                "instruction",
            ),
            (
                serde_json::json!({ "worker": "w", "title": "T", "instruction": "I", "background": false }),
                "read_only",
            ),
            (
                serde_json::json!({ "worker": "w", "title": "T", "instruction": "I", "background": false, "read_only": false, "extra": 1 }),
                "extra",
            ),
        ] {
            let detail = parse_arguments(&refused.to_string()).unwrap_err();
            assert!(detail.contains(why), "{detail}");
        }
    }
}
