//! The context of a conversation's attachments (ADR-0046, decision 3;
//! ADR-0049, decision 1): before the first model call of a reply to an
//! owner's message that asks for it, the run asks the providers of the
//! attachments that the message gathers from for what applies to the
//! message, with the owner's earlier messages, without a model slot, and
//! stores what they give as parts of the message. A message gathers from
//! the conversation's home, on every message, and from the attachments that
//! it names with "@"; other messages leave the search to the model's tools.
//! Stop ends the gathering as it ends every wait.

use mannie_store::{Change, Durability};
use mannie_tools::result_limit;
use mannie_types::context::Attachment;
use mannie_types::run::RunActivity;

use super::{Ending, RunActor};
use crate::attachments::{earlier_for_request, providers_of};
use crate::budget::budget_bytes;
use crate::calls::Setup;

/// What a run gathers for: the attachments that the message gathers from,
/// the owner's message and the owner's earlier messages, newest first.
pub(super) struct DueContext {
    attachments: Vec<Attachment>,
    message: String,
    earlier: Vec<String>,
}

impl RunActor {
    /// The context that the run gathers before its first model call, if
    /// any: when the run answers an owner's message in a conversation with a
    /// home, or one whose message names attachments with "@", and the
    /// message holds no context yet. A retry reuses what an earlier run of
    /// the turn gathered, a wake-up's request is Mannie's note, and a task's
    /// run gathers nothing.
    pub(super) async fn context_due(&self) -> Result<Option<DueContext>, Ending> {
        if self.child.is_some() || self.assigned.is_some() {
            return Ok(None);
        }
        let store = &self.runtime.inner.store;
        let Some(summary) = store.conversation_summary(&self.conversation).await? else {
            return Err(Ending::Gone);
        };
        if summary.attachments.is_empty() {
            return Ok(None);
        }
        let Some(message) = store.turn_request(&self.turn).await? else {
            return Ok(None);
        };
        if store.turn_holds_context(&self.turn).await? {
            return Ok(None);
        }
        let named = store.turn_mention_keys(&self.turn).await?;
        let home = summary.home.as_ref();
        let attachments: Vec<Attachment> = summary
            .attachments
            .into_iter()
            .filter(|attachment| {
                let is_home = home.is_some_and(|home| {
                    home.provider == attachment.provider && home.key == attachment.key
                });
                let is_named = named.iter().any(|(provider, key)| {
                    *provider == attachment.provider && *key == attachment.key
                });
                is_home || is_named
            })
            .collect();
        if attachments.is_empty() {
            return Ok(None);
        }
        let earlier = earlier_for_request(store.earlier_requests(&self.turn).await?);
        Ok(Some(DueContext {
            attachments,
            message,
            earlier,
        }))
    }

    /// Asks the providers, all at once, and stores what each gave as a part
    /// of the owner's message. They share the room of one tool result.
    pub(super) async fn gather_context(
        &mut self,
        setup: &Setup,
        due: DueContext,
    ) -> Result<(), Ending> {
        let providers = providers_of(&due.attachments);
        let gathering = self.activity(RunActivity::GatheringContext { providers });
        self.apply(vec![gathering], Durability::Normal).await?;
        let runtime = self.runtime.clone();
        let conversation = self.conversation.clone();
        let room = result_limit(budget_bytes(setup.budget));
        let parts = self
            .wait(async move {
                runtime
                    .gather_context(
                        &conversation,
                        &due.attachments,
                        &due.message,
                        &due.earlier,
                        room,
                    )
                    .await
            })
            .await?;
        let preparing = self.activity(RunActivity::Preparing);
        self.apply(
            vec![
                Change::AddContext {
                    run_id: self.run.clone(),
                    turn_id: self.turn.clone(),
                    parts,
                },
                preparing,
            ],
            Durability::Full,
        )
        .await?;
        Ok(())
    }
}
