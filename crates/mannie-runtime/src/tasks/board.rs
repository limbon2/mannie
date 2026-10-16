//! The board of a parent's run (ADR-0024, decision 7): each of its children
//! publishes its latest state there, and the parent's run, the only writer of
//! the parent's conversation, stores what changed in the tasks' parts.
//!
//! Publishing replaces a child's state and never waits, so a child never
//! blocks on its parent, and the parent always reads the latest state of
//! each child, its end included. On a worker, the run of a task that the
//! master assigned publishes the same state to the worker's assigned tasks
//! instead, which the master follows (ADR-0028, decision 1).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mannie_store::TaskEnding;
use mannie_types::clock::UtcMillis;
use mannie_types::ids::TaskId;
use mannie_types::run::RunFailure;
use mannie_types::tasks::{ChildApproval, TaskState};
use tokio::sync::Notify;

use crate::assigned::AssignedTasks;

/// What a child's task has ended with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChildEnd {
    /// The task's ending, with the child's report.
    pub(crate) ending: TaskEnding,
    /// When it ended.
    pub(crate) ended_at: UtcMillis,
}

/// A child's state, as its parent mirrors it in the task's part.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ChildState {
    /// Whether the child holds a slot and works, with when it started.
    pub(crate) started_at: Option<UtcMillis>,
    /// The activity of the child's current tool call.
    pub(crate) activity: Option<String>,
    /// How many tool calls the child made for the task.
    pub(crate) tool_calls: u32,
    /// The child's call that waits for the owner.
    pub(crate) approval: Option<ChildApproval>,
    /// On a worker: whether the task's run waits for its credential lease.
    pub(crate) waits_for_lease: bool,
    /// How the task ended, once it ended.
    pub(crate) end: Option<ChildEnd>,
    /// For an executor's task: the executor's plugin stopped, and the run
    /// lets the task go, which may still run on its target (ADR-0031,
    /// decision 10).
    pub(crate) let_go: bool,
}

impl ChildState {
    /// The task's state as the parent's part shows it.
    pub(crate) fn task_state(&self) -> TaskState {
        match (&self.end, self.started_at) {
            (Some(end), _) => end.ending.state,
            (None, Some(_)) => TaskState::Running,
            (None, None) => TaskState::Accepted,
        }
    }
}

#[derive(Debug, Default)]
struct Entry {
    state: ChildState,
    /// Whether the parent has not read the latest state yet.
    changed: bool,
}

/// The board of one parent's run.
#[derive(Debug, Default)]
pub(crate) struct TaskBoard {
    entries: Mutex<HashMap<TaskId, Entry>>,
    changed: Notify,
}

impl TaskBoard {
    fn entries(&self) -> MutexGuard<'_, HashMap<TaskId, Entry>> {
        // Every critical section only replaces entries, so the map stays
        // consistent even if a holder panicked.
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Changes a child's state, and lets the parent know.
    pub(crate) fn publish(&self, task: &TaskId, change: impl FnOnce(&mut ChildState)) {
        {
            let mut entries = self.entries();
            let entry = entries.entry(task.clone()).or_default();
            // A task that ended stays ended: a late update of a child that
            // already published its end changes nothing. Neither does one
            // of a task that the run let go.
            if entry.state.end.is_some() || entry.state.let_go {
                return;
            }
            change(&mut entry.state);
            entry.changed = true;
        }
        // The parent may not wait yet; `notify_one` keeps the wake for it.
        self.changed.notify_one();
    }

    /// The states that changed since the parent last read them.
    pub(crate) fn take_changes(&self) -> Vec<(TaskId, ChildState)> {
        let mut entries = self.entries();
        entries
            .iter_mut()
            .filter(|(_, entry)| entry.changed)
            .map(|(task, entry)| {
                entry.changed = false;
                (task.clone(), entry.state.clone())
            })
            .collect()
    }

    /// Waits until a child publishes something. A change published before
    /// this was called wakes it at once.
    pub(crate) async fn changed(&self) {
        self.changed.notified().await;
    }

    /// Forgets a task whose end the parent stored.
    pub(crate) fn forget(&self, task: &TaskId) {
        self.entries().remove(task);
    }
}

/// Where the run of a task publishes the task's state.
#[derive(Debug, Clone)]
pub(crate) enum TaskSink {
    /// The board of the parent's run, for a child agent.
    Board(Arc<TaskBoard>),
    /// The worker's assigned tasks, for a task that the master assigned.
    Assigned(Arc<AssignedTasks>),
}

impl TaskSink {
    fn publish(&self, task: &TaskId, change: impl FnOnce(&mut ChildState)) {
        match self {
            Self::Board(board) => board.publish(task, change),
            Self::Assigned(assigned) => assigned.publish(task, change),
        }
    }
}

/// A task's hold on where its run publishes: its parent's board, or the
/// worker's assigned tasks. When the run ends without having published the
/// task's end, as when its task panics, the guard publishes a failure, so
/// nobody waits for a run that is gone.
#[derive(Debug)]
pub(crate) struct ChildGuard {
    sink: TaskSink,
    task: TaskId,
    clock: Arc<dyn mannie_types::clock::Clock>,
    published_end: bool,
}

impl ChildGuard {
    /// The guard of a task that publishes to `sink`.
    pub(crate) fn new(
        sink: TaskSink,
        task: TaskId,
        clock: Arc<dyn mannie_types::clock::Clock>,
    ) -> Self {
        Self {
            sink,
            task,
            clock,
            published_end: false,
        }
    }

    /// Changes the task's state.
    pub(crate) fn publish(&self, change: impl FnOnce(&mut ChildState)) {
        self.sink.publish(&self.task, change);
    }

    /// Records the version of the task's latest stored state, which the
    /// states that follow carry. Only an assigned task has versions.
    pub(crate) fn set_version(&self, version: u64) {
        if let TaskSink::Assigned(assigned) = &self.sink {
            assigned.set_version(&self.task, version);
        }
    }

    /// Publishes the task's end, once its run stored it.
    pub(crate) fn publish_end(&mut self, ending: TaskEnding) {
        let ended_at = self.clock.now();
        self.sink.publish(&self.task, |state| {
            state.approval = None;
            state.activity = None;
            state.waits_for_lease = false;
            state.end = Some(ChildEnd { ending, ended_at });
        });
        self.published_end = true;
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.published_end {
            return;
        }
        let ending = TaskEnding {
            state: TaskState::Failed,
            report: None,
            failure: Some(RunFailure::Internal),
        };
        self.publish_end(ending);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use mannie_store::TaskEnding;
    use mannie_types::clock::{ManualClock, UtcMillis};
    use mannie_types::ids::TaskId;
    use mannie_types::run::RunFailure;
    use mannie_types::tasks::TaskState;

    use super::{ChildGuard, TaskBoard, TaskSink};

    fn task(last: u32) -> TaskId {
        TaskId::parse(&format!("0199e0f2-0000-7000-a000-{last:012}")).unwrap()
    }

    fn clock() -> Arc<ManualClock> {
        Arc::new(ManualClock::new(UtcMillis::from_millis(1_000)))
    }

    #[tokio::test]
    async fn the_parent_reads_each_childs_latest_state_once() {
        let board = Arc::new(TaskBoard::default());
        let (first, second) = (task(1), task(2));

        board.publish(&first, |state| state.tool_calls = 1);
        board.publish(&first, |state| state.tool_calls = 2);
        board.publish(&second, |state| {
            state.activity = Some("Reading".to_owned());
        });
        tokio::time::timeout(Duration::from_secs(5), board.changed())
            .await
            .unwrap();
        let mut changes = board.take_changes();
        changes.sort_by(|left, right| left.0.cmp(&right.0));

        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].1.tool_calls, 2);
        assert_eq!(changes[1].1.activity.as_deref(), Some("Reading"));
        assert_eq!(board.take_changes(), Vec::new());
    }

    #[tokio::test]
    async fn an_ended_task_ignores_late_updates() {
        let board = Arc::new(TaskBoard::default());
        let mut guard = ChildGuard::new(TaskSink::Board(Arc::clone(&board)), task(1), clock());

        guard.publish_end(TaskEnding {
            state: TaskState::Reported,
            report: Some("Done.".to_owned()),
            failure: None,
        });
        guard.publish(|state| state.tool_calls = 9);

        let changes = board.take_changes();
        assert_eq!(changes.len(), 1);
        let state = &changes[0].1;
        assert_eq!(state.task_state(), TaskState::Reported);
        assert_eq!(state.tool_calls, 0);
    }

    #[tokio::test]
    async fn a_child_that_ends_without_its_end_fails_on_the_board() {
        let board = Arc::new(TaskBoard::default());
        let guard = ChildGuard::new(TaskSink::Board(Arc::clone(&board)), task(1), clock());
        guard.publish(|state| state.started_at = Some(UtcMillis::from_millis(5)));

        drop(guard);

        let changes = board.take_changes();
        let end = changes[0].1.end.clone().unwrap();
        assert_eq!(end.ending.state, TaskState::Failed);
        assert_eq!(end.ending.failure, Some(RunFailure::Internal));
    }
}
