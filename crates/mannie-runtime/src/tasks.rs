//! Tasks of child agents (docs/features/child-agents/tasks.md; ADR-0024): the
//! task tools that the runtime offers, the pool of slots that limits the
//! children that run at once, the board through which children publish their
//! state to their parent's run, the rendering of reports, and the settings.
//!
//! The run actor drives tasks (`actor/tasks.rs` for the parent's side,
//! `actor/child.rs` for the child's): the task tools act on runs, not on the
//! machine, so they are not tools of the registry.

mod board;
mod report;
mod settings;
mod slots;
mod tools;

pub(crate) use board::{ChildEnd, ChildGuard, ChildState, TaskBoard, TaskSink};
pub(crate) use report::{Doer, render_report, reported_numbers, returned_reports, task_result};
pub(crate) use slots::{ChildSlot, ChildSlots};
pub(crate) use tools::{
    DELEGATE_TASK, FOLLOW_UP_TASK, START_TASK, TaskCall, WAIT_FOR_TASKS, check_instruction,
    definitions, delegate_definition, parse_call,
};

/// The task tools, as agent definitions name them. `delegate_task` is
/// offered only while workers are paired.
pub const TASK_TOOLS: [&str; 4] = [START_TASK, FOLLOW_UP_TASK, WAIT_FOR_TASKS, DELEGATE_TASK];
