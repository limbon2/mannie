//! The commands of the system tools feature that need the runtime
//! (docs/features/system-tools/operations-and-approvals.md, contracts): the
//! tools settings, the rules and grants, and archived outputs. Approvals are
//! in `approvals.rs`; a conversation's workspace is its project's folder
//! (`projects.rs`).

use mannie_store::{RuntimeStoreError, SettingsStoreError, ToolsStoreError};
use mannie_tools::output_page;
use mannie_types::ids::ConversationId;
use mannie_types::tools::{
    CommandRule, ConversationGrant, GrantRef, ReadToolOutput, RuleRef, SharedGrant, SharedGrantRef,
    ToolOutputPage, ToolsError, ToolsSettings, UpdateToolsSettings,
};

use crate::Runtime;

fn runtime_store_failure(error: RuntimeStoreError) -> ToolsError {
    match error {
        RuntimeStoreError::ConversationNotFound => ToolsError::ConversationNotFound,
        other => {
            tracing::error!(error = %other, error.code = other.code(), "the data of a conversation could not be read or saved");
            ToolsError::StorageFailed
        }
    }
}

fn tools_store_failure(error: ToolsStoreError) -> ToolsError {
    match error {
        ToolsStoreError::ConversationNotFound => ToolsError::ConversationNotFound,
        other => {
            tracing::error!(error = %other, error.code = other.code(), "the grants or rules could not be read or saved");
            ToolsError::StorageFailed
        }
    }
}

fn settings_failure(error: &SettingsStoreError) -> ToolsError {
    if let SettingsStoreError::NewerVersion { .. } = error {
        return ToolsError::WrittenByNewerVersion;
    }
    tracing::error!(%error, error.code = error.code(), "the tools settings could not be read or saved");
    ToolsError::StorageFailed
}

impl Runtime {
    /// The tools settings.
    pub async fn tools_settings(&self) -> Result<ToolsSettings, ToolsError> {
        self.inner
            .store
            .load_settings::<ToolsSettings>()
            .await
            .map_err(|error| settings_failure(&error))
    }

    /// Saves the tools settings. Tool calls use them from the next call.
    pub async fn update_tools_settings(
        &self,
        input: UpdateToolsSettings,
    ) -> Result<ToolsSettings, ToolsError> {
        let settings = input.into_settings()?;
        self.inner
            .store
            .save_settings(&settings)
            .await
            .map_err(|error| settings_failure(&error))?;
        Ok(settings)
    }

    /// The command rules, the oldest first.
    pub async fn command_rules(&self) -> Result<Vec<CommandRule>, ToolsError> {
        self.inner.store.rules().await.map_err(tools_store_failure)
    }

    /// Deletes a command rule.
    pub async fn delete_command_rule(&self, input: RuleRef) -> Result<(), ToolsError> {
        let deleted = self
            .inner
            .store
            .delete_rule(&input.rule_id)
            .await
            .map_err(tools_store_failure)?;
        if !deleted {
            return Err(ToolsError::RuleNotFound);
        }
        tracing::info!(rule = %input.rule_id, "deleted a command rule");
        Ok(())
    }

    /// The grants of a conversation, the oldest first.
    pub async fn conversation_grants(
        &self,
        conversation: &ConversationId,
    ) -> Result<Vec<ConversationGrant>, ToolsError> {
        self.inner
            .store
            .grants(conversation)
            .await
            .map_err(tools_store_failure)
    }

    /// Revokes a grant of a conversation. Tool calls ask again from the next
    /// call.
    pub async fn revoke_conversation_grant(&self, input: GrantRef) -> Result<(), ToolsError> {
        let revoked = self
            .inner
            .store
            .revoke_grant(&input.conversation_id, &input.grant_id)
            .await
            .map_err(tools_store_failure)?;
        if !revoked {
            return Err(ToolsError::GrantNotFound);
        }
        tracing::info!(conversation = %input.conversation_id, grant = %input.grant_id, "revoked a grant");
        Ok(())
    }

    /// The grants for a project or for every conversation, the oldest first.
    pub async fn shared_grants(&self) -> Result<Vec<SharedGrant>, ToolsError> {
        self.inner
            .store
            .shared_grants()
            .await
            .map_err(tools_store_failure)
    }

    /// Revokes a grant for a project or for every conversation. Tool calls
    /// ask again from the next call.
    pub async fn revoke_shared_grant(&self, input: SharedGrantRef) -> Result<(), ToolsError> {
        let revoked = self
            .inner
            .store
            .revoke_shared_grant(&input.grant_id)
            .await
            .map_err(tools_store_failure)?;
        if !revoked {
            return Err(ToolsError::GrantNotFound);
        }
        tracing::info!(grant = %input.grant_id, "revoked a shared grant");
        Ok(())
    }

    /// A page of an archived output of a conversation, for the owner.
    pub async fn tool_output(&self, input: ReadToolOutput) -> Result<ToolOutputPage, ToolsError> {
        let archive = self
            .inner
            .store
            .archive(&input.conversation_id, &input.archive_id)
            .await
            .map_err(runtime_store_failure)?
            .ok_or(ToolsError::OutputNotFound)?;
        Ok(output_page(&archive, input.first_line, input.limit))
    }
}
