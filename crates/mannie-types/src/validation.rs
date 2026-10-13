//! IPC types of the desktop app's validation commands (V1, V2, V3). Only
//! validation builds compile them; a release never contains them.

use serde::{Deserialize, Serialize};
use specta::Type;

/// The most simulated runs a channel load may have.
pub const MAX_RUNS: u8 = 16;
/// The most deltas per second one simulated run may stream.
pub const MAX_DELTAS_PER_SECOND: u16 = 2_000;
/// The longest a channel load may last, in milliseconds.
pub const MAX_DURATION_MILLIS: u32 = 60_000;

/// A synthetic streaming load: `runs` simulated runs that each stream
/// `deltas_per_second` deltas for `duration_millis`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelLoad {
    /// How many runs stream at the same time.
    pub runs: u8,
    /// How many deltas each run streams per second.
    pub deltas_per_second: u16,
    /// How long the load lasts, in milliseconds.
    pub duration_millis: u32,
}

impl ChannelLoad {
    /// Checks that the load is within the accepted limits. The webview is
    /// untrusted, so the core checks every request (ADR-0004).
    pub fn check(&self) -> Result<(), ValidationError> {
        let runs_in_range = (1..=MAX_RUNS).contains(&self.runs);
        let rate_in_range = (1..=MAX_DELTAS_PER_SECOND).contains(&self.deltas_per_second);
        let duration_in_range = (1..=MAX_DURATION_MILLIS).contains(&self.duration_millis);
        if runs_in_range && rate_in_range && duration_in_range {
            Ok(())
        } else {
            Err(ValidationError::LoadOutOfRange)
        }
    }
}

/// One synthetic delta.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ValidationDelta {
    /// The simulated run that streamed it.
    pub run: u8,
    /// Its position within its run, from 0.
    pub index: u32,
    /// When the core created it, in milliseconds since the Unix epoch.
    pub created_at: f64,
    /// The delta text.
    pub text: String,
}

/// What the core sent for a channel load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ChannelLoadSummary {
    /// Deltas sent.
    pub deltas: u32,
    /// Batches sent.
    pub batches: u32,
}

/// Why a validation command failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ValidationError {
    /// The requested load is outside the accepted limits.
    #[error("the requested load is outside the accepted limits")]
    LoadOutOfRange,
    /// The webview stopped receiving the channel.
    #[error("the webview stopped receiving the channel")]
    ChannelClosed,
}

#[cfg(test)]
mod tests {
    use super::{ChannelLoad, ValidationError};

    #[test]
    fn loads_outside_the_limits_are_refused() {
        let valid = ChannelLoad {
            runs: 8,
            deltas_per_second: 200,
            duration_millis: 10_000,
        };
        let invalid = [
            ChannelLoad { runs: 0, ..valid },
            ChannelLoad { runs: 17, ..valid },
            ChannelLoad {
                deltas_per_second: 0,
                ..valid
            },
            ChannelLoad {
                deltas_per_second: 2_001,
                ..valid
            },
            ChannelLoad {
                duration_millis: 0,
                ..valid
            },
            ChannelLoad {
                duration_millis: 60_001,
                ..valid
            },
        ];

        assert_eq!(valid.check(), Ok(()));
        for load in invalid {
            assert_eq!(
                load.check(),
                Err(ValidationError::LoadOutOfRange),
                "{load:?}"
            );
        }
    }
}
