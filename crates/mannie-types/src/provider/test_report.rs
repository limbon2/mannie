//! The report of the provider test (docs/features/chat/providers.md, the
//! provider test).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::run::RunFailure;

use super::profile::ServerKind;

/// What the provider test found out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTestReport {
    /// Each check, in the order it ran.
    pub checks: Vec<ProviderCheck>,
    /// The server, when the test recognized it.
    pub server: Option<ServerKind>,
    /// The model's limits as the server reports them, when it does.
    pub reported_limits: Option<ReportedLimits>,
}

/// A model's limits as its server reports them. Servers report the context
/// window more often than the output limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReportedLimits {
    /// The tokens the model reads and writes in one call.
    pub context_window: u32,
    /// The tokens the model writes in one call, if the server says.
    pub output_limit: Option<u32>,
}

/// One check and its outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCheck {
    /// What was checked.
    pub check: CheckKind,
    /// What happened.
    pub outcome: CheckOutcome,
}

/// The checks of the provider test, in the order they run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    /// The stored credential can be read.
    Credential,
    /// The server answers and accepts the credential.
    Server,
    /// The server knows the model.
    Model,
    /// A short request gets a streamed reply.
    Reply,
    /// The server reports token counts.
    Usage,
    /// The server accepts the reasoning effort and returns reasoning.
    Reasoning,
}

/// The outcome of one check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "outcome",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum CheckOutcome {
    /// The check passed.
    Passed,
    /// The check passed with something the owner should know.
    Warning {
        /// What the owner should know.
        note: CheckNote,
    },
    /// The check failed.
    Failed {
        /// Why.
        failure: RunFailure,
    },
    /// The check did not run.
    Skipped {
        /// Why not.
        reason: SkipReason,
    },
}

/// Something the owner should know about a passed check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CheckNote {
    /// The server does not list the model, but may still serve it.
    ModelNotListed,
    /// The server has no list of models, so the model could not be checked.
    ModelListUnavailable,
    /// The server reports no token counts, so the context budget relies on
    /// estimates.
    NoUsageReported,
    /// The server accepted the reasoning effort, but returned no reasoning.
    NoReasoningReturned,
}

/// Why a check did not run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// The profile has no credential.
    NoCredential,
    /// An earlier check failed.
    EarlierCheckFailed,
    /// The profile sets no reasoning effort.
    NoReasoningEffort,
    /// The server has no list of models to look the model up in.
    NoModelList,
}

#[cfg(test)]
mod tests {
    use super::{CheckKind, CheckOutcome, ProviderCheck, SkipReason};

    #[test]
    fn outcomes_serialize_with_their_name() {
        let check = ProviderCheck {
            check: CheckKind::Credential,
            outcome: CheckOutcome::Skipped {
                reason: SkipReason::NoCredential,
            },
        };

        assert_eq!(
            serde_json::to_value(&check).unwrap(),
            serde_json::json!({
                "check": "credential",
                "outcome": { "outcome": "skipped", "reason": "no_credential" },
            })
        );
    }
}
