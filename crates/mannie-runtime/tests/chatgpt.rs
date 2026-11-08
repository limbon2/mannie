//! The ChatGPT account (ADR-0062): its sign-in, its tokens in the vault, one
//! refresh at a time, a token for every call of a profile on the plan, and
//! the end of a session. A fixture server plays OpenAI's sign-in; the fake
//! model plays the Responses API.

// Integration tests are always built with cfg(test); saying so lets Clippy
// treat the helpers below as test code.
#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::points::executors::{Executors, TaskExecutor};
use mannie_plugin_sdk::services::credentials::LendError;
use mannie_provider::testing::{FixtureResponse, FixtureServer, Step};
use mannie_provider::{ProviderError, ProviderEvent};
use mannie_runtime::testing::{
    FAKE_EXECUTOR_KIND, FAKE_EXECUTOR_TOOL, FakeExecutor, Harness, HarnessOptions,
    TEST_CHATGPT_CLIENT, add_plugin, chatgpt_endpoints_at, chatgpt_tokens, credentials,
    plan_profile, reply, sign_in_chatgpt, tool_calls,
};
use mannie_types::chat::{SendMessage, StartConversation, StartedConversation};
use mannie_types::provider::{
    ApiInterface, ChatGptAccountState, CheckKind, CheckOutcome, CredentialChange, ProfileAccess,
    ProfileDraft, ReasoningEffort,
};
use mannie_types::run::{FinishReason, RunFailure, RunState};
use mannie_types::transcript::PartContent;

async fn plan_harness() -> Harness {
    Harness::with_options(HarnessOptions {
        interface: ApiInterface::Responses,
        ..HarnessOptions::default()
    })
    .await
}

async fn start(harness: &Harness, text: &str) -> StartedConversation {
    harness
        .runtime
        .start_conversation(StartConversation::new(text))
        .await
        .unwrap()
}

/// The failure of the run that answered the turn at `position`.
fn failure_at(
    transcript: &mannie_runtime::testing::Transcript,
    position: u32,
) -> Option<RunFailure> {
    transcript
        .parts(position)
        .iter()
        .find_map(|part| match &part.content {
            PartContent::Error { failure } => Some(failure.clone()),
            _ => None,
        })
}

/// The references of the vault's ChatGPT parts.
fn parts(harness: &Harness) -> Vec<String> {
    harness
        .vault
        .references()
        .into_iter()
        .filter(|reference| reference.starts_with("provider/chatgpt/"))
        .collect()
}

#[tokio::test]
async fn a_sign_in_stores_its_tokens_in_parts_and_shows_the_account() {
    let harness = plan_harness().await;
    let server = FixtureServer::start().await.unwrap();

    sign_in_chatgpt(
        &harness.runtime,
        &server,
        "user-1",
        "access-1-planted-0123456789",
        "refresh-1-planted-0123456789",
        false,
    )
    .await;

    let view = harness.runtime.chatgpt_account_view();
    assert_eq!(view.state, ChatGptAccountState::SignedIn);
    assert_eq!(view.email.as_deref(), Some("user-1@example.com"));
    assert!(view.known);
    assert!(!view.notice_seen);
    assert_eq!(parts(&harness), ["provider/chatgpt/1-0"]);
    let bundle = harness.vault.credential("provider/chatgpt/1-0").unwrap();
    assert!(bundle.contains("access-1-planted-0123456789"));
    assert!(bundle.contains("refresh-1-planted-0123456789"));
    let protected = harness.vault.protected();
    assert!(
        protected
            .iter()
            .any(|secret| secret == "access-1-planted-0123456789")
    );
    assert!(
        protected
            .iter()
            .any(|secret| secret == "refresh-1-planted-0123456789")
    );
    let requests = server.requests();
    assert_eq!(requests[0].path, "/api/accounts/oauth/token");
    assert!(String::from_utf8_lossy(&requests[0].body).contains(TEST_CHATGPT_CLIENT));
    assert_eq!(requests[1].path, "/.well-known/jwks.json");
}

#[tokio::test]
async fn each_call_on_the_plan_carries_the_accounts_token() {
    let harness = plan_harness().await;
    let server = FixtureServer::start().await.unwrap();
    sign_in_chatgpt(
        &harness.runtime,
        &server,
        "user-1",
        "access-1",
        "refresh-1",
        false,
    )
    .await;
    plan_profile(&harness.runtime, "gpt-6.1-sol", false).await;
    harness.model.push_reply(reply("Hello."));

    let started = start(&harness, "Hi").await;
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    assert_eq!(
        transcript.run(&started.run_id).unwrap().state,
        RunState::Completed
    );
    assert_eq!(harness.model.credentials(), [Some("access-1".to_owned())]);
}

#[tokio::test]
async fn a_token_that_expires_soon_is_refreshed_before_the_call_and_stored_in_new_parts() {
    let harness = plan_harness().await;
    let server = FixtureServer::start().await.unwrap();
    sign_in_chatgpt(
        &harness.runtime,
        &server,
        "user-1",
        "access-1",
        "refresh-1",
        false,
    )
    .await;
    plan_profile(&harness.runtime, "gpt-6.1-sol", false).await;
    harness.runtime.expire_chatgpt_token();
    let endpoints = chatgpt_endpoints_at(&server);
    server.respond(chatgpt_tokens(&endpoints, "access-2", "refresh-2", None));
    harness.model.push_reply(reply("Hello."));

    let started = start(&harness, "Hi").await;
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    assert_eq!(harness.model.credentials(), [Some("access-2".to_owned())]);
    let refresh = &server.requests()[2];
    assert_eq!(refresh.path, "/api/accounts/oauth/token");
    let form = String::from_utf8_lossy(&refresh.body).into_owned();
    assert!(form.contains("grant_type=refresh_token"), "{form}");
    assert!(form.contains("refresh_token=refresh-1"), "{form}");
    assert_eq!(parts(&harness), ["provider/chatgpt/2-0"]);
    let bundle = harness.vault.credential("provider/chatgpt/2-0").unwrap();
    assert!(bundle.contains("refresh-2"));
}

#[tokio::test]
async fn concurrent_calls_share_one_refresh() {
    let harness = plan_harness().await;
    let server = FixtureServer::start().await.unwrap();
    sign_in_chatgpt(
        &harness.runtime,
        &server,
        "user-1",
        "access-1",
        "refresh-1",
        false,
    )
    .await;
    plan_profile(&harness.runtime, "gpt-6.1-sol", false).await;
    harness.runtime.expire_chatgpt_token();
    let endpoints = chatgpt_endpoints_at(&server);
    let mut slow = chatgpt_tokens(&endpoints, "access-2", "refresh-2", None);
    slow.chunk_delay = Duration::from_millis(200);
    server.respond(slow);
    harness.model.push_reply(reply("One."));
    harness.model.push_reply(reply("Two."));

    let first = start(&harness, "First").await;
    let second = start(&harness, "Second").await;
    harness
        .wait_for_run(&first.conversation_id, &first.run_id)
        .await;
    harness
        .wait_for_run(&second.conversation_id, &second.run_id)
        .await;

    assert_eq!(
        harness.model.credentials(),
        [Some("access-2".to_owned()), Some("access-2".to_owned())]
    );
    assert_eq!(
        server.requests().len(),
        3,
        "one sign-in, its keys, one refresh"
    );
}

#[tokio::test]
async fn a_refused_token_is_refreshed_once_and_the_call_is_made_again() {
    let harness = plan_harness().await;
    let server = FixtureServer::start().await.unwrap();
    sign_in_chatgpt(
        &harness.runtime,
        &server,
        "user-1",
        "access-1",
        "refresh-1",
        false,
    )
    .await;
    plan_profile(&harness.runtime, "gpt-6.1-sol", false).await;
    let endpoints = chatgpt_endpoints_at(&server);
    server.respond(chatgpt_tokens(&endpoints, "access-2", "refresh-2", None));
    harness
        .model
        .push_reply(vec![Step::Error(ProviderError::Permanent(
            RunFailure::InvalidCredential,
        ))]);
    harness.model.push_reply(reply("Hello."));

    let started = start(&harness, "Hi").await;
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    assert_eq!(
        transcript.run(&started.run_id).unwrap().state,
        RunState::Completed
    );
    assert_eq!(
        harness.model.credentials(),
        [Some("access-1".to_owned()), Some("access-2".to_owned())]
    );
}

#[tokio::test]
async fn a_token_refused_twice_ends_the_run_and_keeps_the_account() {
    let harness = plan_harness().await;
    let server = FixtureServer::start().await.unwrap();
    sign_in_chatgpt(
        &harness.runtime,
        &server,
        "user-1",
        "access-1",
        "refresh-1",
        false,
    )
    .await;
    plan_profile(&harness.runtime, "gpt-6.1-sol", false).await;
    let endpoints = chatgpt_endpoints_at(&server);
    server.respond(chatgpt_tokens(&endpoints, "access-2", "refresh-2", None));
    for _refusal in 0..2 {
        harness
            .model
            .push_reply(vec![Step::Error(ProviderError::Permanent(
                RunFailure::InvalidCredential,
            ))]);
    }

    let started = start(&harness, "Hi").await;
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    assert_eq!(
        transcript.run(&started.run_id).unwrap().state,
        RunState::Failed
    );
    assert_eq!(
        failure_at(&transcript, 0),
        Some(RunFailure::ChatGptSignInNeeded)
    );
    assert_eq!(
        harness.runtime.chatgpt_account_view().state,
        ChatGptAccountState::SignedIn
    );
}

#[tokio::test]
async fn a_refresh_that_fails_for_good_ends_the_session() {
    let harness = plan_harness().await;
    let server = FixtureServer::start().await.unwrap();
    sign_in_chatgpt(
        &harness.runtime,
        &server,
        "user-1",
        "access-1",
        "refresh-1",
        false,
    )
    .await;
    plan_profile(&harness.runtime, "gpt-6.1-sol", false).await;
    harness.runtime.expire_chatgpt_token();
    server.respond(FixtureResponse::json(
        400,
        r#"{"error":"refresh_token_reused","error_description":"used"}"#,
    ));

    let started = start(&harness, "Hi").await;
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    assert_eq!(
        failure_at(&transcript, 0),
        Some(RunFailure::ChatGptSignInNeeded)
    );
    assert_eq!(
        harness.runtime.chatgpt_account_view().state,
        ChatGptAccountState::SignInNeeded
    );
    assert_eq!(parts(&harness), Vec::<String>::new());
    assert_eq!(harness.model.credentials(), Vec::<Option<String>>::new());
}

#[tokio::test]
async fn a_refresh_that_fails_for_now_is_retried_and_keeps_the_tokens() {
    let harness = plan_harness().await;
    let server = FixtureServer::start().await.unwrap();
    sign_in_chatgpt(
        &harness.runtime,
        &server,
        "user-1",
        "access-1",
        "refresh-1",
        false,
    )
    .await;
    plan_profile(&harness.runtime, "gpt-6.1-sol", false).await;
    harness.runtime.expire_chatgpt_token();
    let endpoints = chatgpt_endpoints_at(&server);
    server.respond(FixtureResponse::json(503, r#"{"error":"unavailable"}"#));
    server.respond(chatgpt_tokens(&endpoints, "access-2", "refresh-2", None));
    harness.model.push_reply(reply("Hello."));

    let started = start(&harness, "Hi").await;
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    assert_eq!(
        transcript.run(&started.run_id).unwrap().state,
        RunState::Completed
    );
    assert_eq!(harness.model.credentials(), [Some("access-2".to_owned())]);
}

#[tokio::test]
async fn a_usage_limit_is_never_retried() {
    let harness = plan_harness().await;
    let server = FixtureServer::start().await.unwrap();
    sign_in_chatgpt(
        &harness.runtime,
        &server,
        "user-1",
        "access-1",
        "refresh-1",
        false,
    )
    .await;
    plan_profile(&harness.runtime, "gpt-6.1-sol", false).await;
    harness
        .model
        .push_reply(vec![Step::Error(ProviderError::Permanent(
            RunFailure::ChatGptUsageLimitReached,
        ))]);
    harness.model.push_reply(reply("Not reached."));

    let started = start(&harness, "Hi").await;
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    assert_eq!(
        failure_at(&transcript, 0),
        Some(RunFailure::ChatGptUsageLimitReached)
    );
    assert_eq!(harness.model.remaining_replies(), 1);
}

#[tokio::test]
async fn signing_out_revokes_the_session_and_forgets_the_tokens() {
    let harness = plan_harness().await;
    let server = FixtureServer::start().await.unwrap();
    sign_in_chatgpt(
        &harness.runtime,
        &server,
        "user-1",
        "access-1",
        "refresh-1",
        false,
    )
    .await;
    plan_profile(&harness.runtime, "gpt-6.1-sol", false).await;
    server.respond(FixtureResponse::json(200, ""));

    let report = harness.runtime.sign_out_chatgpt().await.unwrap();

    assert!(report.revoked);
    let revoke = &server.requests()[2];
    assert_eq!(revoke.path, "/api/accounts/oauth/revoke");
    assert!(String::from_utf8_lossy(&revoke.body).contains("token=refresh-1"));
    assert_eq!(parts(&harness), Vec::<String>::new());
    let view = harness.runtime.chatgpt_account_view();
    assert_eq!(view.state, ChatGptAccountState::SignedOut);
    assert!(view.known, "the registration stays for the next sign-in");

    let started = start(&harness, "Hi").await;
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    assert_eq!(
        failure_at(&transcript, 0),
        Some(RunFailure::ChatGptSignInNeeded)
    );
}

#[tokio::test]
async fn an_unconfirmed_revocation_still_forgets_the_tokens() {
    let harness = plan_harness().await;
    let server = FixtureServer::start().await.unwrap();
    sign_in_chatgpt(
        &harness.runtime,
        &server,
        "user-1",
        "access-1",
        "refresh-1",
        false,
    )
    .await;
    server.respond(FixtureResponse::json(400, r#"{"error":"invalid_request"}"#));

    let report = harness.runtime.sign_out_chatgpt().await.unwrap();

    assert!(!report.revoked);
    assert_eq!(parts(&harness), Vec::<String>::new());
    assert_eq!(
        harness.runtime.chatgpt_account_view().state,
        ChatGptAccountState::SignedOut
    );
}

#[tokio::test]
async fn another_account_never_receives_the_reasoning_of_the_first() {
    let harness = plan_harness().await;
    let server = FixtureServer::start().await.unwrap();
    sign_in_chatgpt(
        &harness.runtime,
        &server,
        "user-1",
        "access-1",
        "refresh-1",
        false,
    )
    .await;
    plan_profile(&harness.runtime, "gpt-6.1-sol", false).await;
    let item =
        r#"{"type":"reasoning","id":"rs_1","encrypted_content":"opaque-of-user-1","summary":[]}"#;
    harness.model.push_reply(vec![
        Step::Event(ProviderEvent::ProviderItem(item.to_owned())),
        Step::Event(ProviderEvent::TextDelta("One.".to_owned())),
        Step::Event(ProviderEvent::Finished(FinishReason::Stop)),
    ]);
    let started = start(&harness, "First").await;
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    harness.model.push_reply(reply("Same account."));
    let same = harness
        .runtime
        .send_message(SendMessage::new(started.conversation_id.clone(), "Again"))
        .await
        .unwrap();
    harness
        .wait_for_run(&started.conversation_id, &same.run_id)
        .await;

    sign_in_chatgpt(
        &harness.runtime,
        &server,
        "user-2",
        "access-b",
        "refresh-b",
        true,
    )
    .await;
    harness.model.push_reply(reply("Other account."));
    let other = harness
        .runtime
        .send_message(SendMessage::new(started.conversation_id.clone(), "And now"))
        .await
        .unwrap();
    harness
        .wait_for_run(&started.conversation_id, &other.run_id)
        .await;

    let requests = harness.model.reply_requests();
    let carries_item =
        |index: usize| format!("{:?}", requests[index].messages).contains("opaque-of-user-1");
    assert!(carries_item(1), "the same account gets its reasoning back");
    assert!(!carries_item(2), "another account never gets it");
    assert_eq!(
        harness.model.credentials().last(),
        Some(&Some("access-b".to_owned()))
    );
}

#[tokio::test]
async fn the_test_of_a_plan_profile_reads_the_plan_catalog_and_records_no_server() {
    let harness = plan_harness().await;
    let server = FixtureServer::start().await.unwrap();
    sign_in_chatgpt(
        &harness.runtime,
        &server,
        "user-1",
        "access-1",
        "refresh-1",
        false,
    )
    .await;
    let profile = plan_profile(&harness.runtime, "gpt-6.1-sol", false).await;
    server.respond(FixtureResponse::json(
        200,
        r#"{"models":[{"slug":"gpt-6.1-sol","display_name":"GPT-6.1 Sol","visibility":"list"}]}"#,
    ));
    harness.model.push_reply(reply("ready"));

    let report = harness.runtime.test_profile(&profile.id).await.unwrap();

    let outcome = |check: CheckKind| {
        report
            .checks
            .iter()
            .find(|entry| entry.check == check)
            .map(|entry| entry.outcome.clone())
    };
    assert_eq!(outcome(CheckKind::Credential), Some(CheckOutcome::Passed));
    assert_eq!(outcome(CheckKind::Server), Some(CheckOutcome::Passed));
    assert_eq!(outcome(CheckKind::Model), Some(CheckOutcome::Passed));
    assert_eq!(outcome(CheckKind::Reply), Some(CheckOutcome::Passed));
    assert_eq!(report.server, None);
    let catalog = &server.requests()[2];
    assert_eq!(catalog.path, "/v1/models");
    assert_eq!(catalog.header("authorization"), Some("Bearer access-1"));
    let stored = harness.runtime.list_profiles().await.unwrap();
    assert!(stored.iter().all(|profile| profile.server.is_none()));
}

#[tokio::test]
async fn the_catalog_lists_the_plans_models_for_the_profile_form() {
    let harness = plan_harness().await;
    let server = FixtureServer::start().await.unwrap();
    sign_in_chatgpt(
        &harness.runtime,
        &server,
        "user-1",
        "access-1",
        "refresh-1",
        false,
    )
    .await;
    server.respond(FixtureResponse::json(
        200,
        r#"{"models":[{"slug":"gpt-6.1-sol","display_name":"GPT-6.1 Sol","visibility":"list"},{"slug":"codex-6","visibility":"list"}]}"#,
    ));

    let models = harness.runtime.chatgpt_models().await.unwrap();

    let names: Vec<(&str, &str)> = models
        .iter()
        .map(|model| (model.slug.as_str(), model.display_name.as_str()))
        .collect();
    assert_eq!(
        names,
        [("gpt-6.1-sol", "GPT-6.1 Sol"), ("codex-6", "codex-6")]
    );
}

#[tokio::test]
async fn a_plan_profile_takes_no_credential_and_its_endpoint_from_the_core() {
    let harness = plan_harness().await;
    let server = FixtureServer::start().await.unwrap();
    harness
        .runtime
        .set_chatgpt_endpoints(chatgpt_endpoints_at(&server));

    let profile = plan_profile(&harness.runtime, "gpt-6.1-sol", true).await;

    assert_eq!(profile.interface, ApiInterface::Responses);
    assert_eq!(profile.base_url, server.base_url().to_string());
    assert!(!profile.has_credential);
}

/// A harness with tools and the fake executor's plugin on.
async fn harness_with_executor() -> (Harness, Arc<FakeExecutor>) {
    let harness = Harness::with_options(HarnessOptions {
        interface: ApiInterface::Responses,
        use_tools: true,
        ..HarnessOptions::default()
    })
    .await;
    let executor = FakeExecutor::new();
    let contribution: Arc<dyn TaskExecutor> = Arc::<FakeExecutor>::clone(&executor);
    add_plugin(
        &harness.runtime,
        FAKE_EXECUTOR_KIND,
        Contributions::new().add::<Executors>(contribution),
    )
    .unwrap();
    (harness, executor)
}

#[tokio::test]
async fn a_run_on_the_plan_is_offered_no_executor_and_an_endpoint_run_is() {
    let (harness, _executor) = harness_with_executor().await;
    let server = FixtureServer::start().await.unwrap();
    harness.model.push_reply(reply("Endpoint."));
    let first = start(&harness, "First").await;
    harness
        .wait_for_run(&first.conversation_id, &first.run_id)
        .await;
    sign_in_chatgpt(
        &harness.runtime,
        &server,
        "user-1",
        "access-1",
        "refresh-1",
        false,
    )
    .await;
    plan_profile(&harness.runtime, "gpt-6.1-sol", true).await;
    harness.model.push_reply(reply("Plan."));

    let second = start(&harness, "Second").await;
    harness
        .wait_for_run(&second.conversation_id, &second.run_id)
        .await;

    let requests = harness.model.reply_requests();
    let offers_executor = |index: usize| {
        requests[index]
            .tools
            .iter()
            .any(|tool| tool.name == FAKE_EXECUTOR_TOOL)
    };
    assert!(offers_executor(0), "an endpoint run gives executors tasks");
    assert!(!offers_executor(1), "a run on the plan does not");
    assert_ne!(requests[1].tools.len(), 0, "it keeps its other tools");
}

#[tokio::test]
async fn a_profile_that_switched_to_the_plan_lends_nothing() {
    let (harness, executor) = harness_with_executor().await;
    let server = FixtureServer::start().await.unwrap();
    let give = serde_json::json!({
        "activity": "Giving Build it",
        "target": "box-01",
        "title": "Build it",
        "instruction": "Build the backend.",
        "background": false,
    })
    .to_string();
    harness
        .model
        .push_reply(tool_calls(&[("call_1", FAKE_EXECUTOR_TOOL, &give)]));
    harness.model.push_reply(reply("Done."));
    let _started = start(&harness, "OWNER: build it on the box").await;
    let submission = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            if let Some(submission) = executor.submissions().into_iter().next() {
                return submission;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the executor got its task");
    sign_in_chatgpt(
        &harness.runtime,
        &server,
        "user-1",
        "access-1",
        "refresh-1",
        false,
    )
    .await;
    harness
        .runtime
        .update_profile(
            &harness.profile.id,
            ProfileDraft {
                name: "Test".to_owned(),
                access: ProfileAccess::ChatGptPlan,
                interface: ApiInterface::Responses,
                base_url: String::new(),
                model: "gpt-6.1-sol".to_owned(),
                context_window: 128_000,
                output_limit: 16_384,
                reasoning_effort: ReasoningEffort::ServerDefault,
                use_tools: true,
            },
            CredentialChange::Keep,
        )
        .await
        .unwrap();

    let lent = credentials(&harness.runtime, FAKE_EXECUTOR_KIND)
        .lend(submission.task_id.clone(), harness.profile.id.clone())
        .await;

    assert!(matches!(lent, Err(LendError::NotPermitted)), "{lent:?}");
    assert!(
        harness
            .vault
            .references()
            .iter()
            .all(|reference| !reference.starts_with("provider/0")),
        "the profile's own credential was forgotten when it switched to the plan"
    );
}
