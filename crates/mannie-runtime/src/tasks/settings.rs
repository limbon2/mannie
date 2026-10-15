//! The commands of the child agents settings (docs/features/child-agents/
//! child-agent-activity.md, settings): how many children may run at once on
//! the machine.

use mannie_store::SettingsStoreError;
use mannie_types::tasks::{ChildAgentsError, ChildAgentsSettings, UpdateChildAgentsSettings};

use crate::Runtime;

fn settings_failure(error: &SettingsStoreError) -> ChildAgentsError {
    if let SettingsStoreError::NewerVersion { .. } = error {
        return ChildAgentsError::WrittenByNewerVersion;
    }
    tracing::error!(%error, error.code = error.code(), "the child agents settings could not be read or saved");
    ChildAgentsError::StorageFailed
}

impl Runtime {
    /// The child agents settings.
    pub async fn child_agents_settings(&self) -> Result<ChildAgentsSettings, ChildAgentsError> {
        self.inner
            .store
            .load_settings::<ChildAgentsSettings>()
            .await
            .map_err(|error| settings_failure(&error))
    }

    /// Saves the child agents settings and applies the new limit at once: a
    /// larger one starts waiting children, and a smaller one takes effect as
    /// running children end.
    pub async fn update_child_agents_settings(
        &self,
        input: UpdateChildAgentsSettings,
    ) -> Result<ChildAgentsSettings, ChildAgentsError> {
        let settings = input.into_settings()?;
        let _one_at_a_time = self.inner.child_settings.lock().await;
        self.inner
            .store
            .save_settings(&settings)
            .await
            .map_err(|error| settings_failure(&error))?;
        let limit = settings.parallel_children.get();
        self.inner.child_slots.set_limit(limit);
        tracing::info!(limit, "changed how many child agents may run at once");
        Ok(settings)
    }
}
