//! The mode of a worker's policy ceiling (docs/features/workers/
//! host-and-ceiling.md, the ceiling).

use serde::{Deserialize, Serialize};
use specta::Type;

/// The most that a worker's tools may do. The modes are ordered from the
/// narrowest to the widest, so a task's mode is the smaller of the ceiling's
/// and the one the task asks for.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Type,
)]
#[serde(rename_all = "snake_case")]
pub enum CeilingMode {
    /// Only reads run; every change and every command is refused.
    ReadOnly,
    /// Every change and every command asks the owner through the master.
    #[default]
    Ask,
    /// Changes inside the allowed folders and the commands that the host's
    /// rules allow run without asking; everything else asks.
    Autonomous,
    /// Every access and every command runs without asking, inside and outside
    /// the allowed folders and inside the worker's own folders (ADR-0029).
    FullAccess,
}

impl CeilingMode {
    /// The mode for a task that asks for `narrower`: never wider than this
    /// one.
    #[must_use]
    pub fn narrowed_by(self, narrower: Option<Self>) -> Self {
        narrower.map_or(self, |task| self.min(task))
    }

    /// The mode's name, as the configuration and the logs write it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Ask => "ask",
            Self::Autonomous => "autonomous",
            Self::FullAccess => "full_access",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CeilingMode;

    #[test]
    fn a_task_narrows_the_ceiling_and_never_widens_it() {
        assert_eq!(
            CeilingMode::Autonomous.narrowed_by(Some(CeilingMode::ReadOnly)),
            CeilingMode::ReadOnly
        );
        assert_eq!(
            CeilingMode::Ask.narrowed_by(Some(CeilingMode::Autonomous)),
            CeilingMode::Ask
        );
        assert_eq!(
            CeilingMode::ReadOnly.narrowed_by(Some(CeilingMode::Ask)),
            CeilingMode::ReadOnly
        );
        assert_eq!(
            CeilingMode::Autonomous.narrowed_by(None),
            CeilingMode::Autonomous
        );
        assert_eq!(
            CeilingMode::FullAccess.narrowed_by(Some(CeilingMode::ReadOnly)),
            CeilingMode::ReadOnly
        );
        assert_eq!(
            CeilingMode::Autonomous.narrowed_by(Some(CeilingMode::FullAccess)),
            CeilingMode::Autonomous
        );
    }

    #[test]
    fn modes_serialize_with_their_configuration_names() {
        for mode in [
            CeilingMode::ReadOnly,
            CeilingMode::Ask,
            CeilingMode::Autonomous,
            CeilingMode::FullAccess,
        ] {
            assert_eq!(
                serde_json::to_value(mode).unwrap(),
                serde_json::json!(mode.as_str())
            );
        }
    }
}
