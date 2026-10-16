//! Tasks as the UI shows them (docs/features/child-agents/
//! child-agent-activity.md): the content of a `task` part, and the task
//! conversation that a conversation's summary names.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::clock::UtcMillis;
use crate::ids::{ConversationId, PartId, RunId, TaskId, TurnId};
use crate::run::RunFailure;
use crate::tools::{NotRunReason, ToolCallStatus, ToolCallView};

use super::document::{AwaitedTask, TaskExecutorRef, TaskRefusal, TaskState, TaskWorker};

/// A child's call that waits for the owner, with what the UI needs to render
/// it as a part of the child's conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChildApprovalView {
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
    /// The call, with the request to the owner.
    pub call: ToolCallView,
}

/// A task as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskView {
    /// The task.
    pub task_id: TaskId,
    /// The child's number.
    pub number: u32,
    /// The child's conversation.
    pub conversation_id: ConversationId,
    /// The name of the child's agent definition, such as `explore`.
    pub agent: String,
    /// The task's title. A model wrote it, so it is content.
    pub title: String,
    /// Whether the parent continues while the child works.
    pub background: bool,
    /// Whether the task continues an earlier task of the same child.
    pub follow_up: bool,
    /// Where the task is in its life.
    pub state: TaskState,
    /// The start of the instruction. A model wrote it, so it is content.
    pub instruction: String,
    /// The activity of the child's current tool call, as its model named it.
    pub activity: Option<String>,
    /// How many tool calls the child made for the task.
    pub tool_calls: u32,
    /// The child's call that waits for the owner.
    pub approval: Option<ChildApprovalView>,
    /// The child's report. A model wrote it, so it is content.
    pub report: Option<String>,
    /// Why the task failed.
    pub failure: Option<RunFailure>,
    /// When the child started to work.
    pub started_at: Option<UtcMillis>,
    /// When the task ended.
    pub ended_at: Option<UtcMillis>,
    /// The worker of a task that 0.2.0 stored, kept through 0.3.0; the UI
    /// reads `executor`.
    pub worker: Option<TaskWorker>,
    /// The executor that runs the task, or `None` for a child agent.
    pub executor: Option<TaskExecutorRef>,
    /// Whether the run let the task go while its executor ran it.
    pub detached: bool,
}

/// A `task` part as the UI shows it: the call of a task tool, without its
/// arguments, so that each update of a long-running task stays small.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskCallView {
    /// The task tool, such as `start_task`.
    pub tool: String,
    /// What the model said the call does, in a few words.
    pub activity: Option<String>,
    /// Where the call is in its life.
    pub status: ToolCallStatus,
    /// Why the call never started.
    pub not_run: Option<NotRunReason>,
    /// Why the call assigned no task.
    pub refusal: Option<TaskRefusal>,
    /// The task that a start or a follow-up assigned.
    pub task: Option<TaskView>,
    /// The tasks that a wait waits for.
    pub awaited: Vec<AwaitedTask>,
    /// When the call started.
    pub started_at: Option<UtcMillis>,
    /// When the call ended.
    pub ended_at: Option<UtcMillis>,
}

/// What makes a conversation a task conversation: a child's conversation,
/// which its parent's agent directs (ADR-0024, decision 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TaskConversation {
    /// The parent conversation.
    pub parent_id: ConversationId,
    /// The child's number among the parent's children.
    pub number: u32,
    /// The name of the child's agent definition.
    pub agent: String,
    /// The worker of a task conversation that 0.2.0 stored, kept through
    /// 0.3.0; the UI reads `executor`.
    #[serde(default)]
    pub worker: Option<TaskWorker>,
    /// The executor whose task conversation this is, which serves it, or
    /// `None` for a child agent's, which this machine stores.
    #[serde(default)]
    pub executor: Option<TaskExecutorRef>,
}
