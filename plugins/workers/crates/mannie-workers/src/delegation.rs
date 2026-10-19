//! The port through which the Workers plugin's executor reaches the workers
//! (ADR-0027, decision 1; ADR-0031, decision 4): the hub implements it over
//! the link, and the plugin's tests over an in-process link to test workers.

use std::fmt;

use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use mannie_plugin_sdk::contracts::{
    AssignedTaskState, ChatError, Checkpoint, CheckpointRef, ConversationId, ConversationUpdate,
    DecideApproval, EarlierTurns, LoadEarlierTurns, ReadToolOutput, Secret, TaskAssignment, TaskId,
    ToolOutputPage, ToolsError, WorkerId,
};
use tokio::sync::watch;

use crate::view::WorkerView;

/// Why a worker did not do what the executor asked.
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

/// What the executor asks of the workers. Every call may fail with the
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
