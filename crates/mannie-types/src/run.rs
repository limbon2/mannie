//! Runs: the execution of one turn (docs/architecture/domain-model.md, runs),
//! as the store keeps them and the UI shows them.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::clock::UtcMillis;
use crate::ids::{PartId, ProfileId, RunId, TurnId};

/// The lifecycle state of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// Waiting to start.
    Queued,
    /// Working: preparing, compacting, writing or waiting to retry with a
    /// model slot, or running a tool or waiting for an approval without one.
    Running,
    /// The model finished the reply.
    Completed,
    /// The run ended with a failure that names its cause.
    Failed,
    /// The owner stopped the run.
    Stopped,
    /// The process ended while the run was active.
    Interrupted,
}

impl RunState {
    /// Every state, in lifecycle order.
    pub const ALL: [Self; 6] = [
        Self::Queued,
        Self::Running,
        Self::Completed,
        Self::Failed,
        Self::Stopped,
        Self::Interrupted,
    ];

    /// The state's name as the store keeps it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
            Self::Interrupted => "interrupted",
        }
    }

    /// The state with this name.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == name)
    }

    /// Whether the run has not ended yet.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }

    /// Whether the owner can retry the turn after a run in this state.
    pub fn is_retryable(self) -> bool {
        matches!(self, Self::Failed | Self::Stopped | Self::Interrupted)
    }
}

/// What a running run is doing, for the status line of the reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RunActivity {
    /// Waiting for one of the limited model slots.
    WaitingForSlot,
    /// Assembling the context for the next model call.
    Preparing,
    /// Summarizing older turns into a checkpoint.
    Compacting,
    /// Receiving the model's reply.
    Writing,
    /// Waiting before the next attempt after a transient failure.
    Retrying {
        /// The attempt that comes next, counting from 1.
        attempt: u32,
        /// The last attempt Mannie makes.
        max_attempts: u32,
        /// When the next attempt starts.
        next_attempt_at: UtcMillis,
        /// Why the previous attempt failed.
        cause: RunFailure,
    },
    /// Waiting for the owner to decide about a tool call. The run holds no
    /// model slot meanwhile.
    AwaitingApproval {
        /// The tool call's part.
        part_id: PartId,
    },
    /// Running a tool, without a model slot.
    RunningTool {
        /// The tool call's part.
        part_id: PartId,
        /// When the tool started.
        started_at: UtcMillis,
    },
    /// Waiting for child agents to end their tasks, without a model slot
    /// (docs/features/child-agents/tasks.md, lanes).
    WaitingForTasks {
        /// How many tasks the run waits for.
        count: u32,
        /// How many of them run on workers (docs/features/workers/
        /// worker-activity.md).
        #[serde(default)]
        workers: u32,
    },
    /// On a worker: waiting for the credential that the master leases to the
    /// task, without a model slot (docs/features/workers/
    /// credential-leases.md).
    WaitingForLease,
}

/// Why a model call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// The model finished its answer.
    Stop,
    /// The answer reached the profile's output limit.
    OutputLimit,
    /// The provider's content filter ended the answer.
    ContentFilter,
    /// Another reason the provider gave.
    Other,
}

/// Token counts that a provider reported for one model call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    /// Tokens of the request, including cached ones.
    pub input: u32,
    /// Tokens of the answer, including reasoning.
    pub output: u32,
    /// Tokens of the request that the provider read from its cache.
    pub cached_input: Option<u32>,
    /// Tokens the model spent on reasoning.
    pub reasoning: Option<u32>,
}

/// How a run ended: its failure, why its last model call ended, the token
/// counts of that call, and whether the server cut a request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunOutcome {
    /// The cause of a failed run.
    pub failure: Option<RunFailure>,
    /// Why the last model call ended, when it ended on its own.
    pub finish: Option<FinishReason>,
    /// The token counts of the last model call, when the provider reported them.
    pub usage: Option<Usage>,
    /// Whether the server reported a shorter request than the run sent, as a
    /// server does that cuts requests beyond its context instead of failing.
    /// The model may then not have read the start of the conversation. Runs
    /// stored before this field existed read it as `false`.
    #[serde(default)]
    pub prompt_cut: bool,
}

/// Why a run failed or retries. The UI shows the message for the code, with
/// what the owner can do (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RunFailure {
    /// The provider refused the credential.
    #[error("the provider refused the credential")]
    InvalidCredential,
    /// The credential has no permission for the model or the endpoint.
    #[error("the credential has no permission for this request")]
    PermissionDenied,
    /// The account's quota or credit is used up.
    #[error("the account's quota is used up")]
    QuotaExceeded,
    /// The server does not know the model.
    #[error("the server does not know the model {model:?}")]
    ModelNotFound {
        /// The model the profile names.
        model: String,
    },
    /// The server has no such endpoint, which usually means a wrong base URL
    /// or API interface.
    #[error("the server has no such endpoint")]
    EndpointNotFound,
    /// The provider limits the request rate.
    #[error("the provider limits the request rate")]
    RateLimited {
        /// When the provider accepts requests again, if it said so.
        retry_at: Option<UtcMillis>,
    },
    /// The server failed with an HTTP status of 500 or above.
    #[error("the server failed with status {status}")]
    ServerError {
        /// The HTTP status.
        status: u16,
    },
    /// The request is longer than the model's context, even after compaction.
    #[error("the request is longer than the model's context")]
    ContextOverflow,
    /// The latest request alone does not fit into the context budget.
    #[error("the latest request alone does not fit into the context budget")]
    RequestTooLarge,
    /// The server refused the request as malformed.
    #[error("the server refused the request with status {status}")]
    BadRequest {
        /// The HTTP status.
        status: u16,
    },
    /// The provider's content filter refused the request or the answer.
    #[error("the provider's content filter refused the request or the answer")]
    ContentFiltered,
    /// The server could not be reached.
    #[error("the server could not be reached")]
    ConnectionFailed,
    /// The server did not answer in time.
    #[error("the server did not answer in time")]
    TimedOut,
    /// The stream ended before the answer was complete.
    #[error("the stream ended before the answer was complete")]
    BrokenStream,
    /// The server sent something that is not a valid answer.
    #[error("the server sent an invalid answer")]
    InvalidResponse,
    /// The server answered with a redirect, which Mannie does not follow so
    /// that a credential never goes elsewhere.
    #[error("the server answered with a redirect")]
    RedirectRefused,
    /// The profile's credential is missing from the credential store.
    #[error("the profile's credential is missing from the credential store")]
    CredentialMissing,
    /// The credential store could not be read.
    #[error("the credential store could not be read")]
    CredentialStoreUnavailable,
    /// The conversation's provider profile no longer exists.
    #[error("the provider profile no longer exists")]
    ProfileMissing,
    /// No provider profile exists.
    #[error("no provider profile exists")]
    NoProfile,
    /// The server refused the request because of its tools, as servers do
    /// whose model has no tool template or that were started without tool
    /// support. The profile's "Use tools" switch turns tools off.
    #[error("the server does not support tools for this model")]
    ToolsUnsupported,
    /// On a worker: the master revoked the credential that it leased to the
    /// task, which ends the task's run (docs/features/workers/
    /// credential-leases.md).
    #[error("the master revoked the task's credential lease")]
    LeaseRevoked,
    /// Mannie failed. The log has the cause.
    #[error("Mannie failed")]
    Internal,
}

impl RunFailure {
    /// The stable code of the failure, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidCredential => "run.invalid_credential",
            Self::PermissionDenied => "run.permission_denied",
            Self::QuotaExceeded => "run.quota_exceeded",
            Self::ModelNotFound { .. } => "run.model_not_found",
            Self::EndpointNotFound => "run.endpoint_not_found",
            Self::RateLimited { .. } => "run.rate_limited",
            Self::ServerError { .. } => "run.server_error",
            Self::ContextOverflow => "run.context_overflow",
            Self::RequestTooLarge => "run.request_too_large",
            Self::BadRequest { .. } => "run.bad_request",
            Self::ContentFiltered => "run.content_filtered",
            Self::ConnectionFailed => "run.connection_failed",
            Self::TimedOut => "run.timed_out",
            Self::BrokenStream => "run.broken_stream",
            Self::InvalidResponse => "run.invalid_response",
            Self::RedirectRefused => "run.redirect_refused",
            Self::CredentialMissing => "run.credential_missing",
            Self::CredentialStoreUnavailable => "run.credential_store_unavailable",
            Self::ProfileMissing => "run.profile_missing",
            Self::NoProfile => "run.no_profile",
            Self::ToolsUnsupported => "run.tools_unsupported",
            Self::LeaseRevoked => "run.lease_revoked",
            Self::Internal => "run.internal",
        }
    }
}

/// A run as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RunView {
    /// The run.
    pub id: RunId,
    /// The turn it executes.
    pub turn_id: TurnId,
    /// The provider profile it uses.
    pub profile_id: ProfileId,
    /// Its lifecycle state.
    pub state: RunState,
    /// What it is doing, while it runs.
    pub activity: Option<RunActivity>,
    /// How it ended, once it ended.
    pub outcome: Option<RunOutcome>,
    /// When it was created.
    pub created_at: UtcMillis,
    /// When it ended.
    pub ended_at: Option<UtcMillis>,
}

#[cfg(test)]
mod tests {
    use super::{RunActivity, RunFailure, RunState};
    use crate::clock::UtcMillis;

    #[test]
    fn states_round_trip_through_their_names() {
        for state in RunState::ALL {
            assert_eq!(RunState::parse(state.as_str()), Some(state));
        }
        assert_eq!(RunState::parse("paused"), None);
    }

    #[test]
    fn only_ended_runs_that_did_not_complete_can_be_retried() {
        let retryable: Vec<_> = RunState::ALL
            .into_iter()
            .filter(|state| state.is_retryable())
            .collect();
        let active: Vec<_> = RunState::ALL
            .into_iter()
            .filter(|state| state.is_active())
            .collect();

        assert_eq!(
            retryable,
            [RunState::Failed, RunState::Stopped, RunState::Interrupted]
        );
        assert_eq!(active, [RunState::Queued, RunState::Running]);
    }

    #[test]
    fn failures_serialize_with_their_code_and_arguments() {
        let failure = RunFailure::RateLimited {
            retry_at: Some(UtcMillis::from_millis(1_000)),
        };

        assert_eq!(
            serde_json::to_value(&failure).unwrap(),
            serde_json::json!({ "code": "rate_limited", "retryAt": 1000 })
        );
        assert_eq!(failure.code(), "run.rate_limited");
        let back: RunFailure =
            serde_json::from_value(serde_json::to_value(&failure).unwrap()).unwrap();
        assert_eq!(back, failure);
    }

    #[test]
    fn activities_serialize_with_their_kind() {
        let retrying = RunActivity::Retrying {
            attempt: 2,
            max_attempts: 6,
            next_attempt_at: UtcMillis::from_millis(5_000),
            cause: RunFailure::ServerError { status: 503 },
        };

        assert_eq!(
            serde_json::to_value(&retrying).unwrap(),
            serde_json::json!({
                "kind": "retrying",
                "attempt": 2,
                "maxAttempts": 6,
                "nextAttemptAt": 5000,
                "cause": { "code": "server_error", "status": 503 },
            })
        );
        assert_eq!(
            serde_json::to_value(RunActivity::WaitingForSlot).unwrap(),
            serde_json::json!({ "kind": "waiting_for_slot" })
        );
        assert_eq!(
            serde_json::to_value(RunActivity::WaitingForTasks {
                count: 2,
                workers: 1
            })
            .unwrap(),
            serde_json::json!({ "kind": "waiting_for_tasks", "count": 2, "workers": 1 })
        );
        // An activity stored before workers reads as waiting for children.
        assert_eq!(
            serde_json::from_value::<RunActivity>(
                serde_json::json!({ "kind": "waiting_for_tasks", "count": 2 })
            )
            .unwrap(),
            RunActivity::WaitingForTasks {
                count: 2,
                workers: 0
            }
        );
        assert_eq!(
            serde_json::to_value(RunActivity::WaitingForLease).unwrap(),
            serde_json::json!({ "kind": "waiting_for_lease" })
        );
    }
}
