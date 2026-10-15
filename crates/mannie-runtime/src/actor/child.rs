//! The side of a run that works on a task of a child agent
//! (docs/features/child-agents/tasks.md): its slot among the children, what
//! it publishes to its parent's run, the confirmation before its report, and
//! the end of its task.

use mannie_store::{Applied, Durability, TaskEnding};
use mannie_types::ids::ConversationId;
use mannie_types::run::{FinishReason, RunActivity, RunFailure, RunState};
use mannie_types::tasks::{ChildApproval, TaskState};
use mannie_types::transcript::ConversationEvent;

use super::{Ending, RunActor, StepCall};
use crate::tasks::{ChildGuard, ChildSlot};
use crate::texts;

/// What ties a child's run to its parent's run.
#[derive(Debug)]
pub(crate) struct ChildLink {
    /// The parent conversation, whose run gave the task.
    pub(crate) parent: ConversationId,
    /// The child's hold on its parent's board.
    pub(crate) guard: ChildGuard,
}

/// What a child's run keeps of its task.
#[derive(Debug)]
pub(crate) struct ChildRun {
    guard: ChildGuard,
    /// The child's slot, once it holds one.
    slot: Option<ChildSlot>,
    /// Whether the child was asked to confirm that it finished.
    confirmed: bool,
}

impl ChildRun {
    pub(super) fn new(guard: ChildGuard) -> Self {
        Self {
            guard,
            slot: None,
            confirmed: false,
        }
    }

    /// Frees the child's slot, then tells the parent how the task ended. An
    /// ending that was not stored leaves the guard to publish a failure.
    pub(super) fn finish(mut self, ending: Option<TaskEnding>) {
        drop(self.slot.take());
        if let Some(ending) = ending {
            self.guard.publish_end(ending);
        }
    }
}

/// How a child's task ends with its run. The report is the text of the
/// child's last model call; a child that wrote nothing ended without one.
pub(super) fn task_ending(state: RunState, failure: Option<&RunFailure>, text: &str) -> TaskEnding {
    let report = text.trim();
    match state {
        RunState::Completed => TaskEnding {
            state: TaskState::Reported,
            report: (!report.is_empty()).then(|| report.to_owned()),
            failure: None,
        },
        RunState::Failed => TaskEnding {
            state: TaskState::Failed,
            report: None,
            failure: Some(failure.cloned().unwrap_or(RunFailure::Internal)),
        },
        RunState::Stopped => TaskEnding {
            state: TaskState::Canceled,
            report: None,
            failure: None,
        },
        RunState::Interrupted | RunState::Queued | RunState::Running => TaskEnding {
            state: TaskState::Interrupted,
            report: None,
            failure: None,
        },
    }
}

/// The position and the creation time of a call's part, from the events
/// that stored it.
fn stored_part(
    applied: &Applied,
    call: &StepCall,
) -> Option<(u32, mannie_types::clock::UtcMillis)> {
    applied.events.iter().find_map(|(_seq, event)| match event {
        ConversationEvent::PartStarted { part }
        | ConversationEvent::PartUpdated { part }
        | ConversationEvent::PartFinished { part }
            if part.id == call.part_id =>
        {
            Some((part.position, part.created_at))
        }
        _ => None,
    })
}

impl RunActor {
    /// Takes a slot among the children on the machine, in the order the
    /// children asked. Stop and Shutdown end the wait.
    pub(super) async fn acquire_child_slot(&mut self) -> Result<(), Ending> {
        let slots = self.runtime.inner.child_slots.clone();
        let slot = if let Some(slot) = slots.try_acquire() {
            slot
        } else {
            let waiting = self.activity(RunActivity::WaitingForSlot);
            self.apply(vec![waiting], Durability::Normal).await?;
            tracing::debug!(run = %self.run, "a child agent waits for a free slot");
            self.wait(slots.acquire()).await?
        };
        if let Some(child) = &mut self.child {
            child.slot = Some(slot);
        }
        Ok(())
    }

    /// Tells the parent that the child started to work on its task.
    pub(super) fn child_started(&self) {
        let now = self.now();
        if let Some(child) = &self.child {
            child.guard.publish(|state| state.started_at = Some(now));
        }
    }

    /// Tells the parent about the child's tool calls: how many it made, and
    /// the activity of the current one, or `None` between calls.
    pub(super) fn publish_progress(&self, activity: Option<String>) {
        let tool_calls = self.tool_calls;
        if let Some(child) = &self.child {
            child.guard.publish(|state| {
                state.tool_calls = tool_calls;
                state.activity = activity;
            });
        }
    }

    /// Tells the parent about the child's call that waits for the owner,
    /// from the batch that stored the request, so that the owner answers it
    /// in the parent's conversation.
    pub(super) fn publish_approval(&self, call: &StepCall, applied: &Applied) {
        let Some(child) = &self.child else {
            return;
        };
        let Some((position, created_at)) = stored_part(applied, call) else {
            tracing::warn!(run = %self.run, part = %call.part_id, "the stored request of a child's call was not among its events");
            return;
        };
        let approval = ChildApproval {
            part_id: call.part_id.clone(),
            turn_id: self.turn.clone(),
            run_id: self.run.clone(),
            position,
            created_at,
            call: call.document.clone(),
            arguments: call.text.clone(),
        };
        child.guard.publish(|state| state.approval = Some(approval));
    }

    /// Tells the parent that no call of the child waits for the owner.
    pub(super) fn withdraw_approval(&self) {
        if let Some(child) = &self.child {
            child.guard.publish(|state| state.approval = None);
        }
    }

    /// Whether a child that answered without a tool call is asked to
    /// confirm that it finished: once in its turn, after an answer that may
    /// continue.
    pub(super) fn confirmation_due(&self, finish: FinishReason) -> bool {
        let may_continue = matches!(finish, FinishReason::Stop | FinishReason::Other);
        may_continue && self.child.as_ref().is_some_and(|child| !child.confirmed)
    }

    /// Asks the child to confirm that it did everything the task asks before
    /// it reports. The note does not count against the continuation notes.
    pub(super) async fn confirm(&mut self) -> Result<(), Ending> {
        if let Some(child) = &mut self.child {
            child.confirmed = true;
        }
        tracing::debug!(run = %self.run, "asking a child agent to confirm its task before it reports");
        self.add_note(texts::CONFIRMATION_NOTE, false).await
    }
}

#[cfg(test)]
mod tests {
    use mannie_types::run::{RunFailure, RunState};
    use mannie_types::tasks::TaskState;

    use super::task_ending;

    #[test]
    fn a_task_ends_as_its_run_does_and_reports_its_last_text() {
        let reported = task_ending(RunState::Completed, None, "\n  Found it in a.rs.  \n");
        let empty = task_ending(RunState::Completed, None, "  ");
        let failed = task_ending(
            RunState::Failed,
            Some(&RunFailure::QuotaExceeded),
            "partial",
        );

        assert_eq!(reported.state, TaskState::Reported);
        assert_eq!(reported.report.as_deref(), Some("Found it in a.rs."));
        assert_eq!(empty.report, None);
        assert_eq!(failed.state, TaskState::Failed);
        assert_eq!(failed.report, None);
        assert_eq!(failed.failure, Some(RunFailure::QuotaExceeded));
        assert_eq!(
            task_ending(RunState::Stopped, None, "x").state,
            TaskState::Canceled
        );
        assert_eq!(
            task_ending(RunState::Interrupted, None, "x").state,
            TaskState::Interrupted
        );
    }
}
