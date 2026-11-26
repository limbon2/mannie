//! Pending approvals (ADR-0012, decision 1, as refined for system tools): a
//! run that waits for the owner registers a one-shot answer here, and
//! `decide_approval` delivers the owner's choice. The run's mailbox keeps
//! carrying Stop and Shutdown only, so a run that waits for an approval still
//! stops at once (docs/features/system-tools/operations-and-approvals.md,
//! approvals). On a worker, a conversation in full access on the master can
//! answer instead of the owner (ADR-0029, decision 5).

use std::collections::BTreeMap;

use mannie_tools::saved_rule;
use mannie_types::ids::{ConversationId, PartId};
use mannie_types::secret::Secret;
use mannie_types::tools::{
    Access, ApprovalChoice, ApprovalReason, ApprovalRequest, ConfigSecretField, DecideApproval,
    DecideConfigApproval, ToolsError,
};
use tokio::sync::oneshot;

use crate::self_management::MAX_CONFIG_SECRET_BYTES;
use crate::{Runtime, lock};

/// Refuses a choice that the request does not offer, so the UI cannot widen
/// what an approval allows. A worker's request offers only Allow once and
/// Deny, whatever the master sends, so its ceiling stays what its
/// configuration says (ADR-0028, decision 5). Browser site consent can last
/// for the run or conversation. Browser control and app configuration changes
/// are one-time choices (ADR-0064, ADR-0081).
fn check_choice(request: &ApprovalRequest, choice: &ApprovalChoice) -> Result<(), ToolsError> {
    if request.reasons.contains(&ApprovalReason::BrowserCredential)
        && !matches!(
            choice,
            ApprovalChoice::AllowForConversation | ApprovalChoice::Deny
        )
    {
        return Err(ToolsError::GrantNotPossible);
    }
    let once = matches!(choice, ApprovalChoice::AllowOnce | ApprovalChoice::Deny);
    if !once {
        let browser_site = request.reasons.contains(&ApprovalReason::BrowserSite);
        let browser_control = request.reasons.contains(&ApprovalReason::BrowserControl);
        let app_config = request.reasons.contains(&ApprovalReason::AppConfig)
            || request.reasons.contains(&ApprovalReason::AppConfigSecurity);
        if request.host.is_some()
            || browser_control
            || app_config
            || browser_site && !matches!(choice, ApprovalChoice::AllowForConversation)
        {
            return Err(ToolsError::GrantNotPossible);
        }
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
#[derive(Debug)]
pub(crate) enum Answer {
    /// The owner's choice.
    Owner(ApprovalChoice),
    /// The owner's one-time decision on an `AppConfig` approval, carrying any
    /// secrets entered directly on the approval card (ADR-0081).
    ConfigApproval {
        /// Whether the owner chose Allow once (`true`) or Deny (`false`).
        allow: bool,
        /// Masked secret values entered on the card, never logged or serialized.
        secrets: BTreeMap<String, Secret>,
    },
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
    config_secrets: Option<Vec<ConfigSecretField>>,
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
        accesses: &[Access],
    ) -> (Waiting, oneshot::Receiver<Answer>) {
        let (answer, receiver) = oneshot::channel();
        let config_secrets = accesses.iter().find_map(|access| match access {
            Access::AppConfig { secret_fields, .. } => Some(secret_fields.clone()),
            _ => None,
        });
        lock(&self.inner.approvals).insert(
            conversation.clone(),
            Pending {
                part_id: part_id.clone(),
                request: request.clone(),
                config_secrets,
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
    /// the request it holds. A call of an executor's task waits on its
    /// target, which gets the decision through the executor (ADR-0027,
    /// decision 4; ADR-0031).
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
            Some(remote) => {
                let executor = remote.tools_executor()?;
                tracing::info!(
                    conversation = %input.conversation_id,
                    part = %input.part_id,
                    kind = executor.kind(),
                    "the owner decided about a tool call of an executor's task"
                );
                executor.decide_approval(&remote.target, input).await
            }
            None => Err(ToolsError::ApprovalNotPending),
        }
    }

    /// Allows once a call that waits on this worker, because the
    /// conversation on the master that delegated its task is in full access
    /// (ADR-0029, decision 5). A host whose configuration refuses that keeps
    /// the call waiting for the owner.
    pub fn allow_by_full_access(&self, input: &DecideApproval) -> Result<(), ToolsError> {
        let machine = self.machine();
        if machine.assigned_agent.is_none() {
            return Err(ToolsError::ApprovalNotPending);
        }
        if !machine.accepts_assigner_full_access {
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

    /// The owner's answer to a pending `AppConfig` approval, optionally
    /// carrying masked secrets entered directly on the approval card
    /// (ADR-0081). Master-only: never forwarded to workers, and never logs or
    /// persists secret values.
    pub fn decide_config_approval(&self, input: DecideConfigApproval) -> Result<(), ToolsError> {
        let mut approvals = lock(&self.inner.approvals);
        let pending = approvals
            .get(&input.conversation_id)
            .filter(|pending| pending.part_id == input.part_id)
            .ok_or(ToolsError::ApprovalNotPending)?;

        if pending.request.digest != input.digest {
            return Err(ToolsError::ApprovalChanged);
        }

        let Some(fields) = &pending.config_secrets else {
            return Err(ToolsError::GrantNotPossible);
        };

        let validated = if input.allow {
            let mut accepted = BTreeMap::new();

            for (key, value) in input.secrets {
                let Some(_spec) = fields.iter().find(|field| field.key == key) else {
                    return Err(ToolsError::SecretInvalid { field: key });
                };

                let raw = value.expose();
                if raw.len() > MAX_CONFIG_SECRET_BYTES || raw.contains('\0') {
                    return Err(ToolsError::SecretInvalid { field: key });
                }

                if !raw.is_empty() {
                    accepted.insert(key, value);
                }
            }

            for field in fields {
                if field.required && !field.provided_in_chat && !accepted.contains_key(&field.key) {
                    return Err(ToolsError::SecretRequired {
                        field: field.key.clone(),
                    });
                }
            }

            accepted
        } else {
            BTreeMap::new()
        };

        let pending = approvals
            .remove(&input.conversation_id)
            .ok_or(ToolsError::ApprovalNotPending)?;

        pending
            .answer
            .send(Answer::ConfigApproval {
                allow: input.allow,
                secrets: validated,
            })
            .map_err(|_stopped| ToolsError::ApprovalNotPending)?;

        tracing::info!(
            conversation = %input.conversation_id,
            part = %input.part_id,
            allow = input.allow,
            "the owner decided about a configuration change"
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
        if matches!(answer, Answer::FullAccess) && pending.config_secrets.is_some() {
            return Err(ToolsError::FullAccessRefused);
        }
        check_choice(&pending.request, &input.choice)?;
        if input.choice == ApprovalChoice::AllowOnce
            && let Some(fields) = &pending.config_secrets
        {
            for field in fields {
                if field.required && !field.provided_in_chat {
                    return Err(ToolsError::SecretRequired {
                        field: field.key.clone(),
                    });
                }
            }
        }
        let pending = approvals
            .remove(&input.conversation_id)
            .ok_or(ToolsError::ApprovalNotPending)?;
        pending
            .answer
            .send(answer)
            .map_err(|_stopped| ToolsError::ApprovalNotPending)
    }
}
