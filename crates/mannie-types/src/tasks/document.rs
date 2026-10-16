//! The task section of a `task` part, as the store keeps it: the record of a
//! task in its parent conversation (docs/features/child-agents/tasks.md, the
//! task part). Fields that later versions add are optional with a default, so
//! older documents keep reading.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::clock::UtcMillis;
use crate::ids::{ConversationId, PartId, RunId, TaskId, TurnId, WorkerId};
use crate::run::RunFailure;
use crate::tools::{ArchiveRef, ToolCallDocument};

use super::view::{ChildApprovalView, TaskCallView, TaskView};

/// The most characters of a task's title.
pub const MAX_TITLE_CHARS: usize = 80;

/// The largest instruction of a task, in bytes.
pub const MAX_INSTRUCTION_BYTES: usize = 64 * 1024;

/// The most characters of an instruction that its parent's part keeps for the
/// owner. The child's conversation holds the whole instruction.
pub const INSTRUCTION_PREVIEW_CHARS: usize = 1_000;

/// The most bytes of a report that its parent's part keeps for the owner. The
/// model reads the report within its result limit, and the child's
/// conversation holds the whole of it.
pub const MAX_REPORT_VIEW_BYTES: usize = 64 * 1024;

/// Where a task is in its life (docs/architecture/domain-model.md, tasks and
/// reports).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// The task is stored, and the child waits for a free slot.
    Accepted,
    /// The child works on the task.
    Running,
    /// The child gave its confirmed report.
    Reported,
    /// The child's run failed.
    Failed,
    /// The child was stopped, by the owner or with its parent.
    Canceled,
    /// Mannie ended while the task ran.
    Interrupted,
}

impl TaskState {
    /// Every state, in lifecycle order.
    pub const ALL: [Self; 6] = [
        Self::Accepted,
        Self::Running,
        Self::Reported,
        Self::Failed,
        Self::Canceled,
        Self::Interrupted,
    ];

    /// The state's name as the store keeps it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Running => "running",
            Self::Reported => "reported",
            Self::Failed => "failed",
            Self::Canceled => "canceled",
            Self::Interrupted => "interrupted",
        }
    }

    /// The state with this name.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == name)
    }

    /// Whether the task has not ended yet.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Accepted | Self::Running)
    }
}

/// Why a call of a task tool assigned no task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "reason",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum TaskRefusal {
    /// No child of the conversation has this number.
    UnknownTask {
        /// The number that the call named.
        number: u32,
    },
    /// The child still works on its latest task.
    StillRunning {
        /// The child's number.
        number: u32,
    },
    /// The reply started all the tasks it may start.
    BudgetUsed,
    /// Mannie ended after the call was stored and before its task was.
    NotStarted,
    /// The worker that a delegation named is not connected
    /// (docs/features/workers/delegation.md).
    WorkerOffline {
        /// The worker's name.
        worker: String,
    },
    /// No paired worker has the name that a delegation gave.
    WorkerUnknown {
        /// The name that the call gave.
        worker: String,
    },
    /// The worker runs a version of Mannie that this one does not work with.
    WorkerIncompatible {
        /// The worker's name.
        worker: String,
    },
    /// The conversation's provider profile uses an address on this computer,
    /// which a worker on another machine cannot reach.
    ProfileNotReachable {
        /// The worker's name.
        worker: String,
    },
    /// A plugin's executor did not take the task (ADR-0031, decision 6). The
    /// part keeps the text that the model read as its result, and the UI
    /// shows the message that the plugin registers for the code.
    Executor {
        /// The executor's kind: its plugin's ID.
        kind: String,
        /// The executor's code for the refusal, such as `worker_offline`.
        code: String,
        /// The name of the target that the call named, if it named one.
        #[serde(default)]
        target: Option<String>,
    },
}

/// The executor of a plugin that runs a task, and the task's target, such as
/// a worker (ADR-0031, decision 6).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskExecutorRef {
    /// The executor's kind: the ID of the plugin that registered it.
    pub kind: String,
    /// The target's ID, as the executor names it.
    pub target_id: String,
    /// The target's name when the task started. The plugin took it from what
    /// its target says about itself, so it is content.
    pub target_name: String,
    /// What the text that the model reads calls the target, such as
    /// "the worker", which comes before the target's name.
    pub noun: String,
}

/// The worker that a task runs on (docs/features/workers/delegation.md).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskWorker {
    /// The worker.
    pub id: WorkerId,
    /// The worker's name when the task started. The worker wrote it, so it
    /// is content.
    pub name: String,
}

/// A call of a child that waits for the owner, as its parent's part mirrors
/// it, so that the owner answers it in the parent conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildApproval {
    /// The call's part in the child's conversation.
    pub part_id: PartId,
    /// The part's turn.
    pub turn_id: TurnId,
    /// The run that made the call.
    pub run_id: RunId,
    /// The part's position in its turn.
    pub position: u32,
    /// When the part was created.
    pub created_at: UtcMillis,
    /// The call's record, with the request to the owner.
    pub call: ToolCallDocument,
    /// The call's arguments as the model wrote them.
    pub arguments: String,
}

impl ChildApproval {
    /// The approval as the UI shows it.
    pub fn view(&self) -> ChildApprovalView {
        ChildApprovalView {
            part_id: self.part_id.clone(),
            turn_id: self.turn_id.clone(),
            run_id: self.run_id.clone(),
            position: self.position,
            created_at: self.created_at,
            call: self.call.view(&self.arguments),
        }
    }
}

/// The state of a task that the master assigned to a worker, as the worker
/// publishes it and the master follows it (docs/features/workers/
/// delegation.md, following tasks). Every change of a worker's tasks has a
/// higher version than the one before, so the master follows them from the
/// version it stored last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignedTaskState {
    /// The task.
    pub task_id: TaskId,
    /// The task's conversation on the worker.
    pub conversation_id: ConversationId,
    /// The version of this state.
    pub version: u64,
    /// Where the task is in its life.
    pub state: TaskState,
    /// When the worker agent started to work.
    #[serde(default)]
    pub started_at: Option<UtcMillis>,
    /// The activity of the worker agent's current tool call.
    #[serde(default)]
    pub activity: Option<String>,
    /// How many tool calls the worker agent made for the task.
    #[serde(default)]
    pub tool_calls: u32,
    /// The call that waits for the owner.
    #[serde(default)]
    pub approval: Option<ChildApproval>,
    /// Whether the task's run waits for its credential lease.
    #[serde(default)]
    pub waits_for_lease: bool,
    /// The worker agent's report, once the task reported.
    #[serde(default)]
    pub report: Option<String>,
    /// Why the task failed.
    #[serde(default)]
    pub failure: Option<RunFailure>,
    /// When the task ended.
    #[serde(default)]
    pub ended_at: Option<UtcMillis>,
}

/// A task that a wait waits for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AwaitedTask {
    /// The child's number.
    pub number: u32,
    /// The task's title.
    pub title: String,
    /// The task's state, as the wait last saw it.
    pub state: TaskState,
}

/// The task that a start or a follow-up assigned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDocument {
    /// The task.
    pub task_id: TaskId,
    /// The child's number among the children of its parent conversation.
    pub number: u32,
    /// The child's conversation.
    pub conversation_id: ConversationId,
    /// The name of the child's agent definition, such as `explore`.
    pub agent: String,
    /// The task's title, for the owner.
    pub title: String,
    /// Whether the parent continues while the child works.
    #[serde(default)]
    pub background: bool,
    /// Whether the task continues an earlier task of the same child.
    #[serde(default)]
    pub follow_up: bool,
    /// Where the task is in its life.
    pub state: TaskState,
    /// The start of the instruction, at most
    /// [`INSTRUCTION_PREVIEW_CHARS`] characters.
    #[serde(default)]
    pub instruction: String,
    /// The activity of the child's current tool call.
    #[serde(default)]
    pub activity: Option<String>,
    /// How many tool calls the child made for the task.
    #[serde(default)]
    pub tool_calls: u32,
    /// The child's call that waits for the owner.
    #[serde(default)]
    pub approval: Option<ChildApproval>,
    /// The child's report, at most [`MAX_REPORT_VIEW_BYTES`] bytes.
    #[serde(default)]
    pub report: Option<String>,
    /// Why the task failed.
    #[serde(default)]
    pub failure: Option<RunFailure>,
    /// When the child started to work.
    #[serde(default)]
    pub started_at: Option<UtcMillis>,
    /// When the task ended.
    #[serde(default)]
    pub ended_at: Option<UtcMillis>,
    /// The worker of a task that 0.2.0 stored. Later versions name the
    /// executor instead, and M5's migration copies a stored worker into it.
    #[serde(default)]
    pub worker: Option<TaskWorker>,
    /// The executor that runs the task, or `None` for a child agent.
    #[serde(default)]
    pub executor: Option<TaskExecutorRef>,
    /// Whether the run let the task go while its executor ran it: the part
    /// ended, and the task may still run there.
    #[serde(default)]
    pub detached: bool,
}

/// The kind that 0.2.0's stored worker tasks and refusals read as: the
/// Workers plugin's executor (ADR-0031, decision 6). The views name it so
/// that the UI reads one shape; the stored fields stay through 0.3.0.
const LEGACY_WORKERS_KIND: &str = "workers";

/// What the text that the model reads called a worker in 0.2.0.
const LEGACY_WORKER_NOUN: &str = "the worker";

impl TaskWorker {
    /// The worker as the target of the Workers plugin's executor.
    fn as_executor(&self) -> TaskExecutorRef {
        TaskExecutorRef {
            kind: LEGACY_WORKERS_KIND.to_owned(),
            target_id: self.id.as_str().to_owned(),
            target_name: self.name.clone(),
            noun: LEGACY_WORKER_NOUN.to_owned(),
        }
    }
}

impl TaskRefusal {
    /// The refusal as the UI reads it: a refusal of a worker that 0.2.0
    /// stored reads as the Workers plugin's executor's code.
    #[must_use]
    pub fn current(&self) -> Self {
        let legacy = |code: &str, worker: &str| Self::Executor {
            kind: LEGACY_WORKERS_KIND.to_owned(),
            code: code.to_owned(),
            target: Some(worker.to_owned()),
        };
        match self {
            Self::WorkerOffline { worker } => legacy("worker_offline", worker),
            Self::WorkerUnknown { worker } => legacy("worker_unknown", worker),
            Self::WorkerIncompatible { worker } => legacy("worker_incompatible", worker),
            Self::ProfileNotReachable { worker } => legacy("profile_not_reachable", worker),
            other => other.clone(),
        }
    }
}

impl TaskDocument {
    /// The task as the UI shows it.
    pub fn view(&self) -> TaskView {
        TaskView {
            task_id: self.task_id.clone(),
            number: self.number,
            conversation_id: self.conversation_id.clone(),
            agent: self.agent.clone(),
            title: self.title.clone(),
            background: self.background,
            follow_up: self.follow_up,
            state: self.state,
            instruction: self.instruction.clone(),
            activity: self.activity.clone(),
            tool_calls: self.tool_calls,
            approval: self.approval.as_ref().map(ChildApproval::view),
            report: self.report.clone(),
            failure: self.failure.clone(),
            started_at: self.started_at,
            ended_at: self.ended_at,
            worker: self.worker.clone(),
            executor: self
                .executor
                .clone()
                .or_else(|| self.worker.as_ref().map(TaskWorker::as_executor)),
            detached: self.detached,
        }
    }
}

/// What a `task` part holds besides its call record (ADR-0024, decision 2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPartDocument {
    /// The task that a start or a follow-up assigned, once it is stored.
    #[serde(default)]
    pub task: Option<TaskDocument>,
    /// The tasks that a wait waits for.
    #[serde(default)]
    pub awaited: Vec<AwaitedTask>,
    /// Why the call assigned no task.
    #[serde(default)]
    pub refusal: Option<TaskRefusal>,
    /// The result as the model reads it, once the model was given one.
    #[serde(default)]
    pub output: Option<String>,
    /// The outputs archived because they did not fit, such as a long report.
    #[serde(default)]
    pub archives: Vec<ArchiveRef>,
}

impl TaskPartDocument {
    /// The part as the UI shows it, with its call record.
    pub fn view(&self, call: &ToolCallDocument) -> TaskCallView {
        TaskCallView {
            tool: call.tool.clone(),
            activity: call.activity.clone(),
            status: call.status,
            not_run: call.not_run.clone(),
            refusal: self.refusal.as_ref().map(TaskRefusal::current),
            task: self.task.as_ref().map(TaskDocument::view),
            awaited: self.awaited.clone(),
            started_at: call.started_at,
            ended_at: call.ended_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AwaitedTask, ChildApproval, TaskDocument, TaskPartDocument, TaskRefusal, TaskState,
    };
    use crate::clock::UtcMillis;
    use crate::ids::{ConversationId, PartId, RunId, TaskId, TurnId};
    use crate::tools::{ToolCallDocument, ToolCallStatus};

    fn id<T>(parse: fn(&str) -> Result<T, crate::ids::InvalidId>, last: u32) -> T {
        parse(&format!("0199e0f2-0000-7000-a000-{last:012}")).unwrap()
    }

    fn task() -> TaskDocument {
        TaskDocument {
            task_id: id(TaskId::parse, 1),
            number: 2,
            conversation_id: id(ConversationId::parse, 3),
            agent: "explore".to_owned(),
            title: "Find where sessions expire".to_owned(),
            background: false,
            follow_up: false,
            state: TaskState::Running,
            instruction: "Find where sessions expire.".to_owned(),
            activity: Some("Reading session.rs".to_owned()),
            tool_calls: 3,
            approval: None,
            report: None,
            failure: None,
            started_at: Some(UtcMillis::from_millis(10)),
            ended_at: None,
            worker: None,
            executor: None,
            detached: false,
        }
    }

    #[test]
    fn states_round_trip_through_their_names_and_only_two_are_active() {
        for state in TaskState::ALL {
            assert_eq!(TaskState::parse(state.as_str()), Some(state));
        }
        assert_eq!(TaskState::parse("paused"), None);
        let active: Vec<_> = TaskState::ALL
            .into_iter()
            .filter(|state| state.is_active())
            .collect();
        assert_eq!(active, [TaskState::Accepted, TaskState::Running]);
    }

    #[test]
    fn a_document_without_later_fields_still_reads() {
        let document: TaskDocument = serde_json::from_value(serde_json::json!({
            "taskId": "0199e0f2-0000-7000-a000-000000000001",
            "number": 1,
            "conversationId": "0199e0f2-0000-7000-a000-000000000002",
            "agent": "general",
            "title": "Fix the expiry",
            "state": "accepted",
        }))
        .unwrap();

        assert!(!document.background);
        assert_eq!(document.tool_calls, 0);
        assert_eq!(document.approval, None);
        let part: TaskPartDocument = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(part, TaskPartDocument::default());
    }

    #[test]
    fn the_view_carries_the_task_and_the_mirrored_call() {
        let mut call = ToolCallDocument::receiving("call_9".to_owned(), "write_file".to_owned());
        call.status = ToolCallStatus::AwaitingApproval;
        let mut document = task();
        document.approval = Some(ChildApproval {
            part_id: id(PartId::parse, 4),
            turn_id: id(TurnId::parse, 5),
            run_id: id(RunId::parse, 6),
            position: 3,
            created_at: UtcMillis::from_millis(20),
            call,
            arguments: r#"{"path":"notes.md"}"#.to_owned(),
        });
        let part = TaskPartDocument {
            task: Some(document),
            ..TaskPartDocument::default()
        };
        let record = ToolCallDocument::receiving("call_1".to_owned(), "start_task".to_owned());

        let view = part.view(&record);

        assert_eq!(view.tool, "start_task");
        let task = view.task.unwrap();
        assert_eq!(task.number, 2);
        let approval = task.approval.unwrap();
        assert_eq!(approval.call.status, ToolCallStatus::AwaitingApproval);
        assert_eq!(approval.call.arguments, r#"{"path":"notes.md"}"#);
    }

    #[test]
    fn a_worker_task_and_refusal_of_0_2_0_read_as_the_workers_executors() {
        let mut document = task();
        document.worker = Some(super::TaskWorker {
            id: crate::ids::WorkerId::parse("0199e0f2-0000-7000-b000-000000000001").unwrap(),
            name: "build-01".to_owned(),
        });
        let record = ToolCallDocument::receiving("call_1".to_owned(), "delegate_task".to_owned());
        let started = TaskPartDocument {
            task: Some(document),
            ..TaskPartDocument::default()
        };
        let refused = TaskPartDocument {
            refusal: Some(TaskRefusal::WorkerOffline {
                worker: "build-01".to_owned(),
            }),
            ..TaskPartDocument::default()
        };

        let executor = started.view(&record).task.unwrap().executor.unwrap();
        let refusal = refused.view(&record).refusal;

        assert_eq!(executor.kind, "workers");
        assert_eq!(executor.target_id, "0199e0f2-0000-7000-b000-000000000001");
        assert_eq!(executor.target_name, "build-01");
        assert_eq!(executor.noun, "the worker");
        assert_eq!(
            refusal,
            Some(TaskRefusal::Executor {
                kind: "workers".to_owned(),
                code: "worker_offline".to_owned(),
                target: Some("build-01".to_owned()),
            })
        );
        assert_eq!(
            TaskRefusal::BudgetUsed.current(),
            TaskRefusal::BudgetUsed,
            "the core's refusals stay as they are"
        );
    }

    #[test]
    fn refusals_and_awaited_tasks_serialize_with_their_names() {
        assert_eq!(
            serde_json::to_value(TaskRefusal::StillRunning { number: 3 }).unwrap(),
            serde_json::json!({ "reason": "still_running", "number": 3 })
        );
        assert_eq!(
            serde_json::to_value(AwaitedTask {
                number: 1,
                title: "Read the tests".to_owned(),
                state: TaskState::Reported,
            })
            .unwrap(),
            serde_json::json!({ "number": 1, "title": "Read the tests", "state": "reported" })
        );
    }
}
