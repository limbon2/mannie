//! The provider test: short checks of a profile, each with its outcome
//! (docs/features/chat/providers.md, the provider test).

use std::time::Duration;

use futures_util::StreamExt;
use mannie_types::provider::{
    ApiInterface, CheckKind, CheckNote, CheckOutcome, ProviderCheck, ProviderTestReport,
    ReasoningEffort, ServerKind, SkipReason,
};
use mannie_types::run::RunFailure;
use mannie_types::secret::Secret;
use reqwest::Url;

use crate::detect::{self, ModelList};
use crate::provider::{Message, ModelRequest, ProviderEvent, Role, Target};
use crate::quirks::quirks_for;
use crate::registry::ProviderRegistry;
use crate::transport::HttpClient;

/// The request of the reply check.
pub const TEST_PROMPT: &str = "Reply with the single word: ready";

/// How long the reply check may take in all.
const REPLY_TIMEOUT: Duration = Duration::from_secs(120);

/// The most tokens the reply check allows. Reasoning models spend some of them
/// before they answer.
const REPLY_OUTPUT_LIMIT: u32 = 4_096;

/// What the test checks.
#[derive(Debug, Clone)]
pub struct TestInput {
    /// The API interface.
    pub interface: ApiInterface,
    /// The base URL.
    pub base_url: Url,
    /// The model.
    pub model: String,
    /// The credential, when the profile has one and it could be read.
    pub credential: Option<Secret>,
    /// The profile's reasoning effort.
    pub reasoning_effort: ReasoningEffort,
    /// The profile's output limit.
    pub output_limit: u32,
    /// The outcome of reading the credential, which the caller checked.
    pub credential_check: CheckOutcome,
}

/// Runs the test.
pub async fn test_profile(
    client: &HttpClient,
    registry: &ProviderRegistry,
    input: TestInput,
) -> ProviderTestReport {
    let mut checks = vec![ProviderCheck {
        check: CheckKind::Credential,
        outcome: input.credential_check.clone(),
    }];
    let credential_failed = matches!(input.credential_check, CheckOutcome::Failed { .. });
    if credential_failed {
        for check in [
            CheckKind::Server,
            CheckKind::Model,
            CheckKind::Reply,
            CheckKind::Usage,
            CheckKind::Reasoning,
        ] {
            checks.push(skipped(check, SkipReason::EarlierCheckFailed));
        }
        return ProviderTestReport {
            checks,
            server: None,
            reported_limits: None,
        };
    }

    let list = detect::list_models(client, &input.base_url, input.credential.as_ref()).await;
    let server = match &list {
        ModelList::Failed(_) => detect::server_from_host(&input.base_url),
        ModelList::Listed(_) | ModelList::Unavailable => {
            Some(detect::detect_server(client, &input.base_url).await)
        }
    };
    let mut reported_limits = None;
    let (server_outcome, model_outcome) = match &list {
        ModelList::Listed(models) => {
            let listed = models.iter().find(|model| model.id == input.model);
            reported_limits = listed.and_then(|model| model.limits);
            let model = if listed.is_some() {
                CheckOutcome::Passed
            } else {
                CheckOutcome::Warning {
                    note: CheckNote::ModelNotListed,
                }
            };
            (CheckOutcome::Passed, model)
        }
        ModelList::Unavailable => (
            CheckOutcome::Warning {
                note: CheckNote::ModelListUnavailable,
            },
            CheckOutcome::Skipped {
                reason: SkipReason::NoModelList,
            },
        ),
        ModelList::Failed(failure) => (
            CheckOutcome::Failed {
                failure: failure.clone(),
            },
            CheckOutcome::Skipped {
                reason: SkipReason::EarlierCheckFailed,
            },
        ),
    };
    let server_failed = matches!(server_outcome, CheckOutcome::Failed { .. });
    checks.push(ProviderCheck {
        check: CheckKind::Server,
        outcome: server_outcome,
    });
    checks.push(ProviderCheck {
        check: CheckKind::Model,
        outcome: model_outcome,
    });
    if server_failed {
        for check in [CheckKind::Reply, CheckKind::Usage, CheckKind::Reasoning] {
            checks.push(skipped(check, SkipReason::EarlierCheckFailed));
        }
        return ProviderTestReport {
            checks,
            server,
            reported_limits,
        };
    }

    let reply = reply_check(registry, &input, server).await;
    checks.extend(reply.checks(input.reasoning_effort));
    ProviderTestReport {
        checks,
        server,
        reported_limits,
    }
}

fn skipped(check: CheckKind, reason: SkipReason) -> ProviderCheck {
    ProviderCheck {
        check,
        outcome: CheckOutcome::Skipped { reason },
    }
}

/// What the reply check saw.
#[derive(Debug, Default)]
struct Reply {
    failure: Option<RunFailure>,
    usage: bool,
    reasoning: bool,
}

impl Reply {
    fn checks(&self, effort: ReasoningEffort) -> Vec<ProviderCheck> {
        if let Some(failure) = &self.failure {
            return vec![
                ProviderCheck {
                    check: CheckKind::Reply,
                    outcome: CheckOutcome::Failed {
                        failure: failure.clone(),
                    },
                },
                skipped(CheckKind::Usage, SkipReason::EarlierCheckFailed),
                skipped(CheckKind::Reasoning, SkipReason::EarlierCheckFailed),
            ];
        }
        let usage = if self.usage {
            CheckOutcome::Passed
        } else {
            CheckOutcome::Warning {
                note: CheckNote::NoUsageReported,
            }
        };
        let reasoning = match (effort.api_name(), self.reasoning) {
            (None, _) => CheckOutcome::Skipped {
                reason: SkipReason::NoReasoningEffort,
            },
            (Some(_), true) => CheckOutcome::Passed,
            (Some(_), false) => CheckOutcome::Warning {
                note: CheckNote::NoReasoningReturned,
            },
        };
        vec![
            ProviderCheck {
                check: CheckKind::Reply,
                outcome: CheckOutcome::Passed,
            },
            ProviderCheck {
                check: CheckKind::Usage,
                outcome: usage,
            },
            ProviderCheck {
                check: CheckKind::Reasoning,
                outcome: reasoning,
            },
        ]
    }
}

async fn reply_check(
    registry: &ProviderRegistry,
    input: &TestInput,
    server: Option<ServerKind>,
) -> Reply {
    let Some(adapter) = registry.adapter(input.interface) else {
        return Reply {
            failure: Some(RunFailure::Internal),
            ..Reply::default()
        };
    };
    let target = Target {
        base_url: input.base_url.clone(),
        credential: input.credential.clone(),
        quirks: quirks_for(input.interface, server),
    };
    let request = ModelRequest {
        model: input.model.clone(),
        instructions: String::new(),
        messages: vec![Message::text(Role::User, TEST_PROMPT)],
        max_output_tokens: input.output_limit.min(REPLY_OUTPUT_LIMIT),
        reasoning_effort: input.reasoning_effort,
    };
    let mut events = adapter.stream(&target, request);
    let mut reply = Reply::default();
    let reading = async {
        while let Some(event) = events.next().await {
            match event {
                Ok(ProviderEvent::Usage(_)) => reply.usage = true,
                Ok(ProviderEvent::ReasoningDelta(_) | ProviderEvent::ProviderItem(_)) => {
                    reply.reasoning = true;
                }
                Ok(ProviderEvent::Finished(_)) => return,
                Ok(ProviderEvent::TextDelta(_)) => {}
                Err(error) => {
                    reply.failure = Some(error.failure());
                    return;
                }
            }
        }
        reply.failure = Some(RunFailure::BrokenStream);
    };
    if tokio::time::timeout(REPLY_TIMEOUT, reading).await.is_err() {
        reply.failure = Some(RunFailure::TimedOut);
    }
    reply
}
