//! The runtime owns browser site consent and journals each authorized
//! continuation without adding a synthetic tool call to provider history.

#![cfg(test)]

use std::sync::Arc;

use futures_util::FutureExt;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::points::tools::{
    AgentSelector, BrowserAccess, BrowserAction, BrowserControlAccess, PluginTool, Tool,
    ToolAccess, ToolCall, ToolExecution, ToolRefusal, ToolResult, ToolSpec, Tools,
};
use mannie_provider::Content;
use mannie_runtime::testing::{
    Harness, HarnessOptions, SeenPart, Transcript, add_plugin, reply, tool_calls,
};
use mannie_store::StoredSiteCredential;
use mannie_types::chat::{SendMessage, StartConversation};
use mannie_types::ids::{ConversationId, PartId};
use mannie_types::tools::{
    Access, ApprovalChoice, ApprovalReason, ApprovalRequest, DecideApproval, Decision, GrantRef,
    GrantScope, NotRunReason, Refusal, ToolCallStatus, ToolsError, UpdateToolsSettings,
};
use mannie_types::transcript::PartContent;
use serde::Deserialize;
use serde_json::json;

const TOOL: &str = "test_browser_navigate";
const PLUGIN: &str = "test-browser";
const START: &str = "http://127.0.0.1:8000";
const DESTINATION: &str = "http://127.0.0.1:8001";
const CREDENTIAL: &str = "0199e0f2-0000-7000-8000-000000000001";

#[derive(Deserialize)]
struct Visit {
    origin: String,
    redirect: Option<String>,
    credential: Option<String>,
}

#[derive(Debug)]
struct Browser;

impl Tool for Browser {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            TOOL,
            "Visits a test origin.",
            json!({
                "type": "object",
                "properties": {
                    "origin": {"type": "string"},
                    "redirect": {"type": "string"},
                    "credential": {"type": "string"}
                },
                "required": ["origin"],
                "additionalProperties": false
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        async move {
            assert!(call.run_id.is_some());
            let visit: Visit = serde_json::from_str(&call.arguments).unwrap();
            if let Some(reference) = visit.credential {
                let site = browser_access(visit.origin, BrowserAction::Interact);
                let credential = site
                    .clone()
                    .with_credential(reference, "Account password".to_owned());
                return Ok(vec![
                    ToolAccess::BrowserAction(site),
                    ToolAccess::BrowserAction(credential),
                ]);
            }
            Ok(vec![ToolAccess::BrowserAction(browser_access(
                visit.origin,
                BrowserAction::Navigate,
            ))])
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        async move {
            let visit: Visit = serde_json::from_str(&execution.arguments).unwrap();
            let ticket = execution.browser.unwrap();
            assert_eq!(ticket.conversation_id(), &execution.conversation_id);
            if let Some(reference) = visit.credential {
                assert_eq!(ticket.plugin_id(), "browser");
                let site = browser_access(visit.origin, BrowserAction::Interact);
                assert!(ticket.permits(&site));
                assert!(
                    ticket.permits(&site.with_credential(reference, "Account password".to_owned()))
                );
                return ToolResult::new("The credential was allowed.");
            }
            assert_eq!(ticket.plugin_id(), PLUGIN);
            assert!(ticket.permits(&browser_access(visit.origin, BrowserAction::Navigate)));
            if let Some(redirect) = visit.redirect
                && !ticket
                    .authorize(browser_access(redirect, BrowserAction::ContinueNavigation))
                    .await
            {
                return ToolResult::new("The destination was denied.");
            }
            ToolResult::new("The request was allowed.")
        }
        .boxed()
    }
}

fn browser_access(origin: String, action: BrowserAction) -> BrowserAccess {
    BrowserAccess::new(
        origin,
        "session-1".to_owned(),
        "action-1".to_owned(),
        action,
    )
}

fn waiting(transcript: &Transcript, turn: u32) -> Option<(PartId, ApprovalRequest)> {
    transcript
        .parts(turn)
        .into_iter()
        .find_map(|part| match part {
            SeenPart {
                id,
                content: PartContent::ToolCall { call },
                ..
            } if call.status == ToolCallStatus::AwaitingApproval => {
                call.request.clone().map(|request| (id, request))
            }
            _ => None,
        })
}

async fn decide(
    harness: &Harness,
    conversation: &ConversationId,
    turn: u32,
    choice: ApprovalChoice,
) -> PartId {
    let transcript = harness
        .wait_until(conversation, |transcript| {
            waiting(transcript, turn).is_some()
        })
        .await;
    let (part, request) = waiting(&transcript, turn).unwrap();
    assert_eq!(request.reasons, vec![ApprovalReason::BrowserSite]);
    assert!(
        matches!(request.grants.as_slice(), [GrantScope::BrowserSite { plugin, origin }]
        if plugin == PLUGIN && [START, DESTINATION].contains(&origin.as_str()))
    );
    assert!(request.rule.is_none());
    harness
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation.clone(),
            part_id: part.clone(),
            digest: request.digest,
            choice,
        })
        .await
        .unwrap();
    harness
        .wait_until(conversation, |transcript| {
            waiting(transcript, turn).is_none_or(|(id, _)| id != part)
        })
        .await;
    part
}

async fn browser_harness() -> Harness {
    browser_harness_for(PLUGIN).await
}

async fn browser_harness_for(plugin: &str) -> Harness {
    let harness = Harness::with_options(HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    })
    .await;
    add_plugin(
        &harness.runtime,
        plugin,
        Contributions::new().add::<Tools>(
            PluginTool::new(Arc::new(Browser), AgentSelector::Conversations).with_browser_actions(),
        ),
    )
    .unwrap();
    harness
        .runtime
        .update_tools_settings(UpdateToolsSettings {
            allow_changes_in_workspace: false,
            command_environment: Vec::new(),
            full_access_for_new_conversations: true,
            wake_on_command_end: true,
        })
        .await
        .unwrap();
    harness
}

async fn save_test_credential(harness: &Harness) {
    assert!(
        harness
            .store
            .stage_browser_site_credential(StoredSiteCredential {
                reference: CREDENTIAL.to_owned(),
                origin: START.to_owned(),
                label: "Account password".to_owned(),
            })
            .await
            .unwrap()
    );
    assert!(
        harness
            .store
            .activate_browser_site_credential(CREDENTIAL)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn credential_consent_survives_turns_and_stays_in_one_conversation() {
    let harness = browser_harness_for("browser").await;
    save_test_credential(&harness).await;
    let fill = json!({"origin": START, "credential": CREDENTIAL}).to_string();
    harness
        .model
        .push_reply(tool_calls(&[("fill", TOOL, &fill)]));
    harness.model.push_reply(reply("Filled."));
    let started = harness
        .runtime
        .start_conversation(StartConversation::new("Fill."))
        .await
        .unwrap();
    let conversation = started.conversation_id;
    let transcript = harness
        .wait_until(&conversation, |transcript| waiting(transcript, 0).is_some())
        .await;
    let (part_id, request) = waiting(&transcript, 0).unwrap();
    assert_eq!(
        request.reasons,
        [
            ApprovalReason::BrowserSite,
            ApprovalReason::BrowserCredential
        ]
    );
    assert!(matches!(request.grants.as_slice(), [
        GrantScope::BrowserSite { .. },
        GrantScope::BrowserCredential { reference, .. },
    ] if reference == CREDENTIAL));
    harness
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation.clone(),
            part_id,
            digest: request.digest,
            choice: ApprovalChoice::AllowForConversation,
        })
        .await
        .unwrap();
    let transcript = harness.wait_for_run(&conversation, &started.run_id).await;
    assert_eq!(
        transcript.tool_calls(0)[0].status,
        ToolCallStatus::Completed
    );

    harness
        .model
        .push_reply(tool_calls(&[("again", TOOL, &fill)]));
    harness.model.push_reply(reply("Filled again."));
    let next = harness
        .runtime
        .send_message(SendMessage::new(conversation.clone(), "Fill again."))
        .await
        .unwrap();
    let transcript = harness.wait_for_run(&conversation, &next.run_id).await;
    assert_eq!(transcript.tool_calls(1)[0].decision, Some(Decision::Grant));

    harness
        .model
        .push_reply(tool_calls(&[("new", TOOL, &fill)]));
    harness.model.push_reply(reply("Denied."));
    let other = harness
        .runtime
        .start_conversation(StartConversation::new("Try the same credential."))
        .await
        .unwrap();
    let transcript = harness
        .wait_until(&other.conversation_id, |transcript| {
            waiting(transcript, 0).is_some()
        })
        .await;
    let (part_id, request) = waiting(&transcript, 0).unwrap();
    assert!(request.reasons.contains(&ApprovalReason::BrowserCredential));
    harness
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: other.conversation_id.clone(),
            part_id,
            digest: request.digest,
            choice: ApprovalChoice::Deny,
        })
        .await
        .unwrap();
    let transcript = harness
        .wait_for_run(&other.conversation_id, &other.run_id)
        .await;
    assert_eq!(transcript.tool_calls(0)[0].status, ToolCallStatus::Denied);
}

#[tokio::test]
async fn full_access_still_asks_once_per_origin_and_journals_a_redirect() {
    let harness = browser_harness().await;
    harness.model.push_reply(tool_calls(&[
        (
            "first",
            TOOL,
            &json!({"origin": START, "redirect": DESTINATION}).to_string(),
        ),
        ("second", TOOL, &json!({"origin": DESTINATION}).to_string()),
    ]));
    harness.model.push_reply(reply("Visited both."));
    let started = harness
        .runtime
        .start_conversation(StartConversation::new("Browse."))
        .await
        .unwrap();
    let conversation = started.conversation_id;
    let first = decide(&harness, &conversation, 0, ApprovalChoice::AllowOnce).await;
    let continued = decide(&harness, &conversation, 0, ApprovalChoice::AllowOnce).await;
    assert_ne!(first, continued);
    let transcript = harness.wait_for_run(&conversation, &started.run_id).await;
    let calls = transcript.tool_calls(0);
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|call| {
        call.status == ToolCallStatus::Completed && call.decision == Some(Decision::BrowserSite)
    }));
    assert!(calls.iter().any(|call| matches!(
        call.accesses.as_slice(),
        [Access::BrowserAction { action: BrowserAction::ContinueNavigation, origin, .. }]
            if origin == DESTINATION
    )));
    let requests = harness.model.requests();
    let model_calls = requests
        .last()
        .unwrap()
        .messages
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|content| match content {
            Content::ToolCall { call_id, .. } => Some(call_id.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(model_calls, ["first", "second"]);
}

#[tokio::test]
async fn a_declined_site_stays_denied_only_for_the_current_run() {
    let harness = browser_harness().await;
    let redirect = json!({"origin": START, "redirect": DESTINATION}).to_string();
    let destination = json!({"origin": DESTINATION}).to_string();
    harness.model.push_reply(tool_calls(&[
        ("visit", TOOL, &redirect),
        ("later", TOOL, &json!({"origin": START}).to_string()),
    ]));
    harness.model.push_reply(tool_calls(&[
        ("direct_retry", TOOL, &destination),
        ("redirect_retry", TOOL, &redirect),
    ]));
    harness.model.push_reply(reply("Declined."));
    let started = harness
        .runtime
        .start_conversation(StartConversation::new("Browse."))
        .await
        .unwrap();
    let conversation = started.conversation_id;
    decide(&harness, &conversation, 0, ApprovalChoice::AllowOnce).await;
    decide(&harness, &conversation, 0, ApprovalChoice::Deny).await;
    let transcript = harness.wait_for_run(&conversation, &started.run_id).await;
    let [initial, skipped, denied, direct_retry, redirect_retry] =
        transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(
        initial.result.unwrap().output,
        "The destination was denied."
    );
    assert_eq!(skipped.not_run, Some(NotRunReason::EarlierCallDenied));
    assert_eq!(denied.status, ToolCallStatus::Denied);
    assert_eq!(
        direct_retry.not_run,
        Some(NotRunReason::Refused {
            refusal: Refusal::BrowserSiteDenied
        }),
    );
    assert_eq!(
        redirect_retry.result.unwrap().output,
        "The destination was denied."
    );

    harness
        .model
        .push_reply(tool_calls(&[("fresh_run", TOOL, &destination)]));
    harness.model.push_reply(reply("Visited."));
    let second_run = harness
        .runtime
        .send_message(SendMessage::new(conversation.clone(), "Browse again."))
        .await
        .unwrap();
    decide(&harness, &conversation, 1, ApprovalChoice::AllowOnce).await;
    let transcript = harness
        .wait_for_run(&conversation, &second_run.run_id)
        .await;
    assert_eq!(
        transcript.tool_calls(1)[0].status,
        ToolCallStatus::Completed
    );
}

#[tokio::test]
async fn a_saved_site_grant_covers_redirects_and_later_runs_until_revoked() {
    let harness = browser_harness().await;
    let redirect = json!({"origin": START, "redirect": DESTINATION}).to_string();
    harness
        .model
        .push_reply(tool_calls(&[("visit", TOOL, &redirect)]));
    harness.model.push_reply(reply("Visited."));
    let started = harness
        .runtime
        .start_conversation(StartConversation::new("Browse."))
        .await
        .unwrap();
    let conversation = started.conversation_id;
    decide(
        &harness,
        &conversation,
        0,
        ApprovalChoice::AllowForConversation,
    )
    .await;
    decide(
        &harness,
        &conversation,
        0,
        ApprovalChoice::AllowForConversation,
    )
    .await;
    let transcript = harness.wait_for_run(&conversation, &started.run_id).await;
    assert_eq!(transcript.tool_calls(0).len(), 2);
    assert!(transcript.tool_calls(0).iter().all(|call| {
        call.status == ToolCallStatus::Completed
            && call.decision == Some(Decision::AllowedForConversation)
    }));
    let grants = harness
        .runtime
        .conversation_grants(&conversation)
        .await
        .unwrap();
    assert_eq!(grants.len(), 2);
    let destination_grant = grants
        .into_iter()
        .find(|grant| matches!(&grant.scope, GrantScope::BrowserSite { origin, .. } if origin == DESTINATION))
        .unwrap();

    let destination = json!({"origin": DESTINATION}).to_string();
    harness.model.push_reply(tool_calls(&[
        ("redirect", TOOL, &redirect),
        ("direct", TOOL, &destination),
    ]));
    harness.model.push_reply(reply("Used saved grants."));
    let second_run = harness
        .runtime
        .send_message(SendMessage::new(conversation.clone(), "Browse again."))
        .await
        .unwrap();
    let transcript = harness
        .wait_for_run(&conversation, &second_run.run_id)
        .await;
    assert_eq!(transcript.tool_calls(1).len(), 2);
    assert!(transcript.tool_calls(1).iter().all(|call| {
        call.status == ToolCallStatus::Completed && call.decision == Some(Decision::Grant)
    }));

    harness
        .runtime
        .revoke_conversation_grant(GrantRef {
            conversation_id: conversation.clone(),
            grant_id: destination_grant.id,
        })
        .await
        .unwrap();
    harness
        .model
        .push_reply(tool_calls(&[("redirect_after_revoke", TOOL, &redirect)]));
    harness.model.push_reply(reply("Allowed once."));
    let third_run = harness
        .runtime
        .send_message(SendMessage::new(
            conversation.clone(),
            "Browse after revocation.",
        ))
        .await
        .unwrap();
    decide(&harness, &conversation, 2, ApprovalChoice::AllowOnce).await;
    let transcript = harness.wait_for_run(&conversation, &third_run.run_id).await;
    assert_eq!(transcript.tool_calls(2).len(), 2);
    assert!(transcript.tool_calls(2).iter().any(|call| matches!(
        call.accesses.as_slice(),
        [Access::BrowserAction { action: BrowserAction::ContinueNavigation, origin, .. }]
            if origin == DESTINATION && call.decision == Some(Decision::BrowserSite)
    )));
}

#[derive(Debug)]
struct Handoff;

impl Tool for Handoff {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "test_browser_observe",
            "Requests browser control before reading.",
            json!({
                "type": "object", "properties": {}, "additionalProperties": false
            }),
        )
    }

    fn prepare(&self, _call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        std::future::ready(Ok(vec![ToolAccess::BrowserControl(control_access())])).boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let ticket = execution.browser.unwrap();
        assert!(ticket.permits_control(&control_access()));
        assert!(!ticket.permits_control(&BrowserControlAccess::new(
            "session-1".to_owned(),
            "2".to_owned()
        )));
        assert!(!ticket.permits(&browser_access(START.to_owned(), BrowserAction::Observe)));
        std::future::ready(ToolResult::new("Control returned. The page was not read.")).boxed()
    }
}

fn control_access() -> BrowserControlAccess {
    BrowserControlAccess::new("session-1".to_owned(), "1".to_owned())
}

#[tokio::test]
async fn handoff_from_a_read_tool_requires_one_time_approval_under_full_access() {
    let harness = browser_harness().await;
    add_plugin(
        &harness.runtime,
        "test-browser-handoff",
        Contributions::new().add::<Tools>(
            PluginTool::new(Arc::new(Handoff), AgentSelector::Conversations).with_browser_reads(),
        ),
    )
    .unwrap();
    harness
        .model
        .push_reply(tool_calls(&[("observe", "test_browser_observe", "{}")]));
    harness.model.push_reply(reply("Control returned."));
    let started = harness
        .runtime
        .start_conversation(StartConversation::new("Observe the browser."))
        .await
        .unwrap();
    let conversation = started.conversation_id;
    let transcript = harness
        .wait_until(&conversation, |transcript| waiting(transcript, 0).is_some())
        .await;
    let (part_id, request) = waiting(&transcript, 0).unwrap();
    assert_eq!(request.reasons, [ApprovalReason::BrowserControl]);
    assert_eq!(request.grants, Vec::<GrantScope>::new());
    assert!(request.rule.is_none());
    let mut input = DecideApproval {
        conversation_id: conversation.clone(),
        part_id,
        digest: request.digest,
        choice: ApprovalChoice::AllowForConversation,
    };
    assert!(matches!(
        harness.runtime.decide_approval(input.clone()).await,
        Err(ToolsError::GrantNotPossible)
    ));
    input.choice = ApprovalChoice::AllowOnce;
    harness.runtime.decide_approval(input).await.unwrap();
    let transcript = harness.wait_for_run(&conversation, &started.run_id).await;
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(call.decision, Some(Decision::AllowedOnce));
    assert!(matches!(
        call.accesses.as_slice(),
        [Access::BrowserControl { .. }]
    ));
    assert_eq!(
        harness
            .runtime
            .conversation_grants(&conversation)
            .await
            .unwrap(),
        Vec::<mannie_types::tools::ConversationGrant>::new()
    );
}
