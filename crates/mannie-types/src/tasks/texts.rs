//! The texts about tasks that the model reads in the tools that start them,
//! the core's `start_task` and an executor's own, such as Workers'
//! `delegate_task` (ADR-0031, decision 4), so that both say the same. Like
//! the runtime's other texts, they are a contract with the model.

/// The argument `title` of a tool that starts a task.
pub const TASK_TITLE_ARGUMENT: &str =
    "A few words for the owner that name the task, such as \"Find the callers of parse_config\".";

/// The result of a follow-up of a task that still runs.
pub fn task_still_running(number: u32) -> String {
    format!(
        "Task {number} is still running. Call wait_for_tasks for its report before you send a \
follow-up."
    )
}
