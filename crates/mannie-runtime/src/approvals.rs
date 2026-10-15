//! Pending approvals (ADR-0012, decision 1, as refined for system tools): a
//! run that waits for the owner registers a one-shot answer here, and
//! `decide_approval` delivers the owner's choice. The run's mailbox keeps
//! carrying Stop and Shutdown only, so a run that waits for an approval still
//! stops at once (docs/features/system-tools/operations-and-approvals.md,
//! approvals).

use mannie_tools::saved_rule;
use mannie_types::ids::{ConversationId, PartId};
use mannie_types::tools::{ApprovalChoice, ApprovalRequest, DecideApproval, ToolsError};
use tokio::sync::oneshot;

use crate::{Runtime, lock};

/// Refuses a choice that the request does not offer, so the UI cannot widen
/// what an approval allows.
fn check_choice(request: &ApprovalRequest, choice: &ApprovalChoice) -> Result<(), ToolsError> {
    match choice {
        ApprovalChoice::AllowOnce | ApprovalChoice::Deny => Ok(()),
        ApprovalChoice::AllowForConversation | ApprovalChoice::AllowAlways => {
            if request.grants.is_empty() {
                Err(ToolsError::GrantNotPossible)
            } else {
                Ok(())
            }
        }
        ApprovalChoice::AllowForProject => {
            if request.project.is_none() {
                Err(ToolsError::NotInAProject)
            } else if request.grants.is_empty() {
                Err(ToolsError::GrantNotPossible)
            } else {
                Ok(())
            }
        }
        ApprovalChoice::SaveRule {
            prefix_words,
            limit_to_folder,
        } => {
            let proposal = request.rule.as_ref().ok_or(ToolsError::RuleNotPossible)?;
            saved_rule(proposal, *prefix_words, *limit_to_folder)?;
            Ok(())
        }
    }
}

/// An approval that a run waits for.
#[derive(Debug)]
pub(crate) struct Pending {
    part_id: PartId,
    request: ApprovalRequest,
    answer: oneshot::Sender<ApprovalChoice>,
}

/// A run's wait for an approval. Dropping it, when the run stops waiting for
/// any reason, withdraws the request.
#[derive(Debug)]
pub(crate) struct Waiting {
    runtime: Runtime,
    conversation: ConversationId,
    part_id: PartId,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        let mut approvals = lock(&self.runtime.inner.approvals);
        let is_this_request = approvals
            .get(&self.conversation)
            .is_some_and(|pending| pending.part_id == self.part_id);
        if is_this_request {
            approvals.remove(&self.conversation);
        }
    }
}

impl Runtime {
    /// Registers a run's question to the owner and returns the wait for the
    /// answer.
    pub(crate) fn ask_owner(
        &self,
        conversation: &ConversationId,
        part_id: &PartId,
        request: &ApprovalRequest,
    ) -> (Waiting, oneshot::Receiver<ApprovalChoice>) {
        let (answer, receiver) = oneshot::channel();
        lock(&self.inner.approvals).insert(
            conversation.clone(),
            Pending {
                part_id: part_id.clone(),
                request: request.clone(),
                answer,
            },
        );
        let waiting = Waiting {
            runtime: self.clone(),
            conversation: conversation.clone(),
            part_id: part_id.clone(),
        };
        (waiting, receiver)
    }

    /// The owner's answer to a pending approval. It counts only for the
    /// request it names by its digest, so a stale or forged decision changes
    /// nothing (T-5). Grants and rules are saved by the run that waits, from
    /// the request it holds.
    pub fn decide_approval(&self, input: DecideApproval) -> Result<(), ToolsError> {
        let mut approvals = lock(&self.inner.approvals);
        let pending = approvals
            .get(&input.conversation_id)
            .filter(|pending| pending.part_id == input.part_id)
            .ok_or(ToolsError::ApprovalNotPending)?;
        if pending.request.digest != input.digest {
            return Err(ToolsError::ApprovalChanged);
        }
        check_choice(&pending.request, &input.choice)?;
        let pending = approvals
            .remove(&input.conversation_id)
            .ok_or(ToolsError::ApprovalNotPending)?;
        tracing::info!(
            conversation = %input.conversation_id,
            part = %input.part_id,
            "the owner decided about a tool call"
        );
        pending
            .answer
            .send(input.choice)
            .map_err(|_stopped| ToolsError::ApprovalNotPending)
    }
}
