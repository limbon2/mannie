//! Pending approvals (ADR-0012, decision 1, as refined for system tools): a
//! run that waits for the owner registers a one-shot answer here, and
//! `decide_approval` delivers the owner's choice. The run's mailbox keeps
//! carrying Stop and Shutdown only, so a run that waits for an approval still
//! stops at once (docs/features/system-tools/operations-and-approvals.md,
//! approvals). On a worker, a conversation in full access on the master can
//! answer instead of the owner (ADR-0029, decision 5).

use mannie_tools::saved_rule;
use mannie_types::ids::{ConversationId, PartId};
use mannie_types::tools::{ApprovalChoice, ApprovalRequest, DecideApproval, ToolsError};
use tokio::sync::oneshot;

use crate::{Runtime, lock};

/// Refuses a choice that the request does not offer, so the UI cannot widen
/// what an approval allows. A worker's request offers only Allow once and
/// Deny, whatever the master sends, so its ceiling stays what its
/// configuration says (ADR-0028, decision 5).
fn check_choice(request: &ApprovalRequest, choice: &ApprovalChoice) -> Result<(), ToolsError> {
    let once = matches!(choice, ApprovalChoice::AllowOnce | ApprovalChoice::Deny);
    if request.host.is_some() && !once {
        return Err(ToolsError::GrantNotPossible);
    }
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

/// The answer to an approval that a run waits for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Answer {
    /// The owner's choice.
    Owner(ApprovalChoice),
    /// On a worker: Allow once from the conversation on the master that
    /// delegated the task, which is in full access. The call records the
    /// decision `full_access` (ADR-0029, decision 5).
    FullAccess,
}

/// An approval that a run waits for.
#[derive(Debug)]
pub(crate) struct Pending {
    part_id: PartId,
    request: ApprovalRequest,
    answer: oneshot::Sender<Answer>,
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
    ) -> (Waiting, oneshot::Receiver<Answer>) {
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
    /// the request it holds. A call of a task on a worker waits on the
    /// worker, which gets the decision through the link (ADR-0027,
    /// decision 4).
    pub async fn decide_approval(&self, input: DecideApproval) -> Result<(), ToolsError> {
        match self.decide_here(&input) {
            Err(ToolsError::ApprovalNotPending) => {}
            decided => return decided,
        }
        let remote = self
            .remote_conversation(&input.conversation_id)
            .await
            .map_err(|error| {
                tracing::error!(%error, error.code = error.code(), "could not read the conversation of a decision");
                ToolsError::StorageFailed
            })?;
        match remote {
            Some((delegation, worker, _summary)) => {
                tracing::info!(
                    conversation = %input.conversation_id,
                    part = %input.part_id,
                    worker = %worker,
                    "the owner decided about a tool call on a worker"
                );
                delegation.decide_approval(&worker, input).await
            }
            None => Err(ToolsError::ApprovalNotPending),
        }
    }

    /// Allows once a call that waits on this worker, because the
    /// conversation on the master that delegated its task is in full access
    /// (ADR-0029, decision 5). A host whose configuration refuses that keeps
    /// the call waiting for the owner.
    pub fn allow_by_full_access(&self, input: &DecideApproval) -> Result<(), ToolsError> {
        let host = self.worker_host().ok_or(ToolsError::ApprovalNotPending)?;
        if !host.accepts_master_full_access {
            return Err(ToolsError::FullAccessRefused);
        }
        if input.choice != ApprovalChoice::AllowOnce {
            return Err(ToolsError::GrantNotPossible);
        }
        self.deliver(input, Answer::FullAccess)?;
        tracing::info!(
            conversation = %input.conversation_id,
            part = %input.part_id,
            "the master's full access allowed a tool call"
        );
        Ok(())
    }

    /// Delivers the owner's decision to a run of this machine that waits for
    /// it.
    fn decide_here(&self, input: &DecideApproval) -> Result<(), ToolsError> {
        self.deliver(input, Answer::Owner(input.choice.clone()))?;
        tracing::info!(
            conversation = %input.conversation_id,
            part = %input.part_id,
            "the owner decided about a tool call"
        );
        Ok(())
    }

    /// Delivers an answer to the request that `input` names by its digest.
    fn deliver(&self, input: &DecideApproval, answer: Answer) -> Result<(), ToolsError> {
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
        pending
            .answer
            .send(answer)
            .map_err(|_stopped| ToolsError::ApprovalNotPending)
    }
}
