//! Data for the end-to-end performance checks (NFR-PERF-04): a long
//! conversation, written straight into the store. Only the `validation`
//! feature compiles this module; the desktop app's validation builds use it.

use mannie_store::RuntimeStoreError;
use mannie_types::ids::ConversationId;

use crate::Runtime;

impl Runtime {
    /// Seeds a conversation of `turns` completed turns, each with the owner's
    /// message and a reply, and adds it to the conversation list.
    pub async fn seed_conversation(&self, turns: u32) -> Result<ConversationId, RuntimeStoreError> {
        let conversation = self.inner.ids.conversation();
        let profile = match self.inner.store.default_profile().await {
            Ok(Some(profile)) => profile.id,
            _ => self.inner.ids.profile(),
        };
        mannie_store::validation::seed_conversation(
            &self.inner.store,
            &conversation,
            &profile,
            turns,
            || self.inner.ids.part().as_str().to_owned(),
        )
        .await?;
        if let Some(summary) = self.inner.store.conversation_summary(&conversation).await? {
            self.inner.hubs.announce(summary).await;
        }
        tracing::info!(conversation = %conversation, turns, "seeded a conversation (validation build)");
        Ok(conversation)
    }
}
