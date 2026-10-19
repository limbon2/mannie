//! The service `executor-host` (ADR-0034, decision 5): what a plugin's task
//! executor uses of the core (ADR-0031, decision 4). That is the rows and
//! states of its tasks, its document on each task's row, the outbox of
//! deletions that wait for its targets, and whether a task's parent
//! conversation is in full access (ADR-0029, decision 5).
//!
//! The core binds the service to the executor kind that equals the plugin's
//! ID, so a plugin names neither its kind nor another plugin's tasks: each
//! method answers only for tasks of that kind, and refuses or ignores the
//! rest as its documentation says.
//!
//! [`ExecutorHostApi::outbox_changes`] returns a watch receiver, a handle
//! that the protocol rules exempt (decision 3), like a stream.

use std::fmt;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::BoxFuture;
use crate::contracts::{AssignedTaskState, ConversationId, TaskId, TaskState};
use crate::service::Service;

/// The service `executor-host`, major 1.
#[derive(Debug)]
pub enum ExecutorHost {}

impl Service for ExecutorHost {
    const NAME: &'static str = "executor-host";
    const MAJOR: u16 = 1;
    type Api = dyn ExecutorHostApi;
}

/// What a plugin's executor uses of the core, bound to the executor kind
/// that equals the plugin's ID.
pub trait ExecutorHostApi: Send + Sync + fmt::Debug {
    /// Publishes a state of one of the executor's tasks: the core stores it
    /// in the task's row and shows it on the board of the run that follows
    /// the task. A state of a task that is another kind's or another
    /// conversation's, of a task that ended, or older than the one stored
    /// changes nothing.
    ///
    /// Returns the task as it was before the state, for a state that the
    /// core took, and `None` for one that changed nothing.
    fn publish_task_state(
        &self,
        state: AssignedTaskState,
    ) -> BoxFuture<'static, Result<Option<ExecutorTask>, HostFailure>>;

    /// Ends a task that its target does not know or can no longer report,
    /// and tells the run that follows it. A task that is not the executor's,
    /// such as another kind's task or an unknown one, is refused with the
    /// code `not_permitted`, and its row and its board stay as they are. A
    /// task that ended already keeps its first ending.
    fn end_task(
        &self,
        task: TaskId,
        state: TaskState,
    ) -> BoxFuture<'static, Result<(), HostFailure>>;

    /// One of the executor's tasks, by its ID. `None` for a task of another
    /// kind, a task that no executor runs, and an unknown task.
    fn task(&self, task: TaskId) -> BoxFuture<'static, Result<Option<ExecutorTask>, HostFailure>>;

    /// The executor's tasks that have not ended, the oldest first.
    fn active_tasks(&self) -> BoxFuture<'static, Result<Vec<ExecutorTask>, HostFailure>>;

    /// Replaces the executor's document of one of its tasks. Returns whether
    /// the task is the executor's: another kind's task keeps its document.
    fn set_document(
        &self,
        task: TaskId,
        document: Option<String>,
    ) -> BoxFuture<'static, Result<bool, HostFailure>>;

    /// The deletions of task conversations that wait for the executor's
    /// targets.
    fn outbox(&self) -> BoxFuture<'static, Result<Vec<OutboxItem>, HostFailure>>;

    /// Forgets deletions that the executor took. Items that are not the
    /// executor's stay in the outbox.
    fn drop_outbox(&self, items: Vec<OutboxItem>) -> BoxFuture<'static, Result<(), HostFailure>>;

    /// Forgets the deletions that wait for one of the executor's targets,
    /// such as a worker that the owner removed (ADR-0031, decision 9).
    /// Returns how many it forgot.
    fn drop_target_outbox(&self, target: String) -> BoxFuture<'static, Result<u64, HostFailure>>;

    /// Changes whenever a deletion joins the outbox, for any kind, so the
    /// executor reads its outbox again.
    fn outbox_changes(&self) -> watch::Receiver<u64>;

    /// Whether the parent of one of the executor's task conversations is in
    /// full access (ADR-0029, decision 5). `false` for every other
    /// conversation, and when the core cannot read it.
    fn parent_has_full_access(&self, conversation: ConversationId) -> BoxFuture<'static, bool>;
}

/// One of the executor's tasks, as the core keeps its row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ExecutorTask {
    /// The task.
    pub id: TaskId,
    /// The task's conversation: the child's conversation on this machine.
    pub conversation_id: ConversationId,
    /// Where the task is in its life.
    pub state: TaskState,
    /// The target that runs the task, as the executor names it.
    pub target: String,
    /// The executor's own document of the task, which the core keeps as it
    /// is.
    pub document: Option<String>,
}

impl ExecutorTask {
    /// A task of the executor, with its target and its document.
    pub fn new(
        id: TaskId,
        conversation_id: ConversationId,
        state: TaskState,
        target: String,
        document: Option<String>,
    ) -> Self {
        Self {
            id,
            conversation_id,
            state,
            target,
            document,
        }
    }
}

/// The deletion of a task conversation that waits for the target that keeps
/// it (ADR-0031, decision 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct OutboxItem {
    /// The target that keeps the conversation.
    pub target: String,
    /// The conversation.
    pub conversation_id: ConversationId,
}

impl OutboxItem {
    /// The deletion of `conversation_id` on `target`.
    pub fn new(target: String, conversation_id: ConversationId) -> Self {
        Self {
            target,
            conversation_id,
        }
    }
}

/// Why the core could not do what the executor asked, as a stable code: the
/// store's own code when the store failed, such as `store.query`, or
/// `not_permitted` for a task of another kind. The core decides the codes. A
/// plugin logs the code and never shows it as text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("the executor host failed with the code {code}")]
pub struct HostFailure {
    code: String,
}

impl HostFailure {
    /// A failure with its code.
    pub fn new(code: impl Into<String>) -> Self {
        Self { code: code.into() }
    }

    /// The failure's code.
    pub fn code(&self) -> &str {
        &self.code
    }
}

#[cfg(test)]
mod tests {
    use super::HostFailure;

    #[test]
    fn a_failure_keeps_its_code_and_names_it() {
        let failure = HostFailure::new("not_permitted");

        assert_eq!(failure.code(), "not_permitted");
        assert_eq!(
            failure.to_string(),
            "the executor host failed with the code not_permitted"
        );
    }
}
