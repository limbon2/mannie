//! The `child_agents` settings section and the errors of the feature's
//! commands (docs/features/child-agents/tasks.md, slots).

use serde::{Deserialize, Deserializer, Serialize};
use specta::Type;

use crate::settings::SettingsSection;

/// How many child agents may run at once, on the whole machine. Deserializing
/// checks the range, so a stored document is valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Type)]
#[serde(transparent)]
pub struct ParallelChildren(u32);

impl ParallelChildren {
    /// The smallest limit.
    pub const MIN: u32 = 1;
    /// The largest limit.
    pub const MAX: u32 = 16;
    /// The limit until the owner chooses another.
    pub const DEFAULT: u32 = 4;

    /// Checks the range.
    pub fn new(limit: u32) -> Result<Self, ChildAgentsError> {
        if (Self::MIN..=Self::MAX).contains(&limit) {
            Ok(Self(limit))
        } else {
            Err(ChildAgentsError::ParallelChildrenInvalid {
                min: Self::MIN,
                max: Self::MAX,
            })
        }
    }

    /// The limit.
    pub fn get(self) -> u32 {
        self.0
    }
}

impl Default for ParallelChildren {
    fn default() -> Self {
        Self(Self::DEFAULT)
    }
}

impl<'de> Deserialize<'de> for ParallelChildren {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let limit = u32::deserialize(deserializer)?;
        Self::new(limit).map_err(serde::de::Error::custom)
    }
}

/// The child agents settings section.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChildAgentsSettings {
    /// How many child agents may run at once.
    pub parallel_children: ParallelChildren,
}

impl SettingsSection for ChildAgentsSettings {
    const KEY: &'static str = "child_agents";
    const VERSION: u32 = 1;
}

/// The argument of `update_child_agents_settings`: the complete new section,
/// checked by the core.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateChildAgentsSettings {
    /// How many child agents may run at once.
    pub parallel_children: u32,
}

impl UpdateChildAgentsSettings {
    /// Checks the request and turns it into the section to store.
    pub fn into_settings(self) -> Result<ChildAgentsSettings, ChildAgentsError> {
        Ok(ChildAgentsSettings {
            parallel_children: ParallelChildren::new(self.parallel_children)?,
        })
    }
}

/// Why a command of the child agents feature failed. The UI shows the
/// message for the code from its catalog (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ChildAgentsError {
    /// The number of child agents at once is outside the range Mannie
    /// accepts.
    #[error("the number of child agents at once must be from {min} to {max}")]
    ParallelChildrenInvalid {
        /// The smallest limit.
        min: u32,
        /// The largest limit.
        max: u32,
    },
    /// The settings were written by a newer version of Mannie, which this
    /// version does not overwrite.
    #[error("the child agents settings were written by a newer version of Mannie")]
    WrittenByNewerVersion,
    /// The store failed. The log has the cause.
    #[error("the data could not be read or saved")]
    StorageFailed,
    /// The core started without its database.
    #[error("the database is not open")]
    StoreUnavailable,
}

impl ChildAgentsError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::ParallelChildrenInvalid { .. } => "child_agents.parallel_children_invalid",
            Self::WrittenByNewerVersion => "child_agents.written_by_newer_version",
            Self::StorageFailed => "child_agents.storage_failed",
            Self::StoreUnavailable => "child_agents.store_unavailable",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ChildAgentsError, ChildAgentsSettings, ParallelChildren, UpdateChildAgentsSettings,
    };

    #[test]
    fn the_limit_is_four_until_the_owner_chooses_and_stays_in_its_range() {
        assert_eq!(ChildAgentsSettings::default().parallel_children.get(), 4);
        assert_eq!(ParallelChildren::new(1).unwrap().get(), 1);
        assert_eq!(ParallelChildren::new(16).unwrap().get(), 16);
        for limit in [0, 17, 1_000] {
            assert_eq!(
                ParallelChildren::new(limit),
                Err(ChildAgentsError::ParallelChildrenInvalid { min: 1, max: 16 })
            );
        }
    }

    #[test]
    fn an_update_becomes_the_section_and_a_stored_limit_out_of_range_does_not_read() {
        let settings = UpdateChildAgentsSettings {
            parallel_children: 8,
        }
        .into_settings()
        .unwrap();

        assert_eq!(settings.parallel_children.get(), 8);
        assert_eq!(
            serde_json::to_value(&settings).unwrap(),
            serde_json::json!({ "parallelChildren": 8 })
        );
        serde_json::from_value::<ChildAgentsSettings>(serde_json::json!({ "parallelChildren": 0 }))
            .unwrap_err();
        UpdateChildAgentsSettings {
            parallel_children: 40,
        }
        .into_settings()
        .unwrap_err();
    }

    #[test]
    fn errors_serialize_with_their_code() {
        assert_eq!(
            serde_json::to_value(ChildAgentsError::ParallelChildrenInvalid { min: 1, max: 16 })
                .unwrap(),
            serde_json::json!({ "code": "parallel_children_invalid", "min": 1, "max": 16 })
        );
        assert_eq!(
            ChildAgentsError::StorageFailed.code(),
            "child_agents.storage_failed"
        );
    }
}
