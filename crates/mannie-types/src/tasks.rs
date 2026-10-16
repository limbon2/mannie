//! The child agents feature's contracts (docs/features/child-agents/): the
//! task section of a `task` part and its view, the task conversation that a
//! summary names, the `child_agents` settings section, and the errors of the
//! feature's commands.
//!
//! A task is one start or follow-up of a child agent's work. The part that
//! assigned it is its record in the parent conversation, and the child works
//! in a task conversation of its own (ADR-0024).

mod assignment;
mod document;
mod settings;
mod view;

pub use assignment::{ProfileSnapshot, TaskAssignment};
pub use document::{
    AssignedTaskState, AwaitedTask, ChildApproval, INSTRUCTION_PREVIEW_CHARS,
    MAX_INSTRUCTION_BYTES, MAX_REPORT_VIEW_BYTES, MAX_TITLE_CHARS, TaskDocument, TaskExecutorRef,
    TaskPartDocument, TaskRefusal, TaskState, TaskWorker,
};
pub use settings::{
    ChildAgentsError, ChildAgentsSettings, ParallelChildren, UpdateChildAgentsSettings,
};
pub use view::{ChildApprovalView, TaskCallView, TaskConversation, TaskView};
