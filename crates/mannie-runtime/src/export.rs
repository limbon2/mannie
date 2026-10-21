//! Full saved conversation records, dispatched through the executor port for
//! remote tasks. No export read executes a tool or changes a conversation.

use std::time::Duration;

use mannie_types::export::{ConversationRecord, ExportError, RECORD_VERSION};
use mannie_types::ids::{ConversationId, ExportId};

use crate::Runtime;

const REMOTE_READ_TIMEOUT: Duration = Duration::from_secs(30);

impl Runtime {
    /// Allocates an export operation ID using the runtime's injected clock.
    pub fn new_export_id(&self) -> ExportId {
        self.inner.ids.export()
    }

    /// A complete record of a local conversation or a task on its executor.
    pub async fn export_record(
        &self,
        conversation: &ConversationId,
    ) -> Result<ConversationRecord, ExportError> {
        if let Some(remote) = self
            .remote_conversation(conversation)
            .await
            .map_err(|_error| ExportError::StorageFailed)?
        {
            let executor = remote
                .executor
                .as_ref()
                .map_err(|_plugin| ExportError::PluginOff)?;
            let mut record = tokio::time::timeout(
                REMOTE_READ_TIMEOUT,
                executor.export_record(&remote.target, conversation),
            )
            .await
            .map_err(|_elapsed| ExportError::HostUnavailable)??;
            if record.version != RECORD_VERSION || record.conversation.id != *conversation {
                return Err(ExportError::InvalidRecord);
            }
            record.conversation =
                crate::hub::localize_summary(&remote.summary, &record.conversation);
            return Ok(record);
        }
        self.inner
            .store
            .conversation_record(conversation)
            .await
            .map_err(|_error| ExportError::StorageFailed)?
            .ok_or(ExportError::ConversationNotFound)
    }

    /// The worker link can read only a conversation assigned by its paired
    /// master. Normal owner conversations on a mixed host are not exposed.
    pub async fn assigned_export_record(
        &self,
        conversation: &ConversationId,
    ) -> Result<ConversationRecord, ExportError> {
        if self.machine().assigned_agent.is_none() {
            return Err(ExportError::ConversationNotFound);
        }
        let task = self
            .inner
            .store
            .latest_task(conversation)
            .await
            .map_err(|_error| ExportError::StorageFailed)?;
        if task.is_none_or(|task| task.assignment.is_none()) {
            return Err(ExportError::ConversationNotFound);
        }
        self.inner
            .store
            .conversation_record(conversation)
            .await
            .map_err(|_error| ExportError::StorageFailed)?
            .ok_or(ExportError::ConversationNotFound)
    }
}
