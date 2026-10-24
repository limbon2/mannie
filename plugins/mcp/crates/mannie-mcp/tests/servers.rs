//! The servers' tools through the core's tool loop, with a scripted model and
//! real MCP servers (plugins/mcp/docs/README.md; ADR-0043): the example
//! program `mcp-test-server` over stdio, in the current protocol era and in
//! the era of 2025, and a server over Streamable HTTP in the test's process,
//! with a secret header or with OAuth through its fake authorization server.
//!
//! Each test checks what the owner and the model see: the approval of a
//! call, grants that cover one tool of one server with any arguments, the
//! result with its images and without the server's secrets, the log, and
//! what a crash, a timeout and a tool that arrives later do.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt as _;
use mannie_mcp::mannie_plugin;
use mannie_mcp_testing::oauth::{OAuthOptions, sign_in_through};
use mannie_mcp_testing::{
    HttpOptions, LATE_TOOL, README_TEXT, SECRET_VARIABLE, legacy, serve_http, server_program,
};
use mannie_plugin_harness::host::{HostOptions, HostedPlugin};
use mannie_plugin_harness::runtime::{
    Harness, HarnessOptions, SeenPart, Transcript, published_tools, reply, tool_calls,
};
use mannie_plugin_harness::types::{
    Access, ApprovalChoice, ApprovalReason, ApprovalRequest, DecideApproval, Decision, GrantScope,
    MentionPlace, MentionRef, PartContent, PartId, RunId, StartConversation, SuggestMentions,
    ToolCallStatus, ToolCallView,
};
use mannie_plugin_sdk::contracts::{ConversationId, ToolServerEndpoint};
use mannie_plugin_sdk::ipc::ChannelMessage;
use mannie_plugin_sdk::testing::FakeSecrets;
use serde_json::{Value, json};

/// The secret that the tests plant in the server's environment.
const PLANTED: &str = "planted-test-token-value";

/// The plugin, on, in a runtime whose conversations use tools.
async fn hosted() -> HostedPlugin {
    hosted_with(&Arc::new(FakeSecrets::new())).await
}

/// The plugin, on, with its secrets in `secrets`.
async fn hosted_with(secrets: &Arc<FakeSecrets>) -> HostedPlugin {
    let core = HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    };
    let provider: Arc<FakeSecrets> = Arc::clone(secrets);
    let options = HostOptions::new().with_core(core).with_provider(provider);
    let hosted = HostedPlugin::new(mannie_plugin(), options).await;
    hosted.turn_on().await;
    hosted
}

/// The example program as a local server, with the planted secret.
fn local(name: &str, arguments: &[&str], timeout_seconds: u32) -> Value {
    json!({
        "name": name,
        "transport": "stdio",
        "program": server_program().display().to_string(),
        "arguments": arguments,
        "secrets": [{ "name": SECRET_VARIABLE, "value": PLANTED }],
        "images": true,
        "instructions": true,
        "timeoutSeconds": timeout_seconds,
    })
}

/// Adds a server and waits until it is connected and its tools reach the
/// runtime. Returns its view.
async fn connect(hosted: &HostedPlugin, server: Value) -> Value {
    let added = hosted
        .invoke("add_server", server)
        .await
        .expect("the server is added");
    let id = added["id"].as_str().expect("a server has an ID").to_owned();
    let prefix = added["prefix"]
        .as_str()
        .expect("a server has a prefix")
        .to_owned();
    let connected = wait_for_view(hosted, &id, |server| server["status"] == "connected").await;
    let echo = format!("mcp_{prefix}_echo");
    tokio::time::timeout(Duration::from_secs(20), async {
        while !published_tools(&hosted.core.runtime).contains(&echo) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the server's tools reach the runtime");
    connected
}

/// Waits until the view of the server `id` in the channel matches.
async fn wait_for_view(hosted: &HostedPlugin, id: &str, matches: impl Fn(&Value) -> bool) -> Value {
    let mut channel = hosted
        .open_channel("follow_servers", Value::Null)
        .await
        .expect("the channel opens");
    let found = tokio::time::timeout(Duration::from_secs(150), async {
        while let Some(message) = channel.next().await {
            let ChannelMessage::Item { item } = message else {
                break;
            };
            let server = item["servers"]
                .as_array()
                .and_then(|servers| servers.iter().find(|server| server["id"] == id))
                .cloned()
                .unwrap_or(Value::Null);
            if matches(&server) {
                return Some(server);
            }
        }
        None
    })
    .await;
    found
        .ok()
        .flatten()
        .expect("the server matches before the channel ends, in time")
}

/// The arguments of a call, with the activity that the core takes out.
fn arguments(value: Value) -> String {
    let mut value = value;
    value["activity"] = json!("Testing");
    value.to_string()
}

async fn start(core: &Harness, text: &str) -> (ConversationId, RunId) {
    let started = core
        .runtime
        .start_conversation(StartConversation::new(text))
        .await
        .expect("the conversation starts");
    (started.conversation_id, started.run_id)
}

/// The tool call of the turn that waits for the owner, if one does.
fn waiting_call(transcript: &Transcript) -> Option<(PartId, ApprovalRequest)> {
    transcript.parts(0).into_iter().find_map(|part| match part {
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

/// Waits until a call waits for the owner, answers it with `choice`, and
/// waits until it no longer waits. Returns the request.
async fn answer(
    core: &Harness,
    conversation: &ConversationId,
    choice: ApprovalChoice,
) -> ApprovalRequest {
    let transcript = core
        .wait_until(conversation, |transcript| {
            waiting_call(transcript).is_some()
        })
        .await;
    let (part_id, request) = waiting_call(&transcript).expect("a call waits for the owner");
    core.runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation.clone(),
            part_id: part_id.clone(),
            digest: request.digest.clone(),
            choice,
        })
        .await
        .expect("the owner's decision is taken");
    core.wait_until(conversation, |transcript| {
        waiting_call(transcript).is_none_or(|(waiting, _request)| waiting != part_id)
    })
    .await;
    request
}

/// The output of a call that ran.
fn output(call: &ToolCallView) -> &str {
    &call.result.as_ref().expect("the call ran").output
}

/// Runs one call of `tool` that the owner allows once, and returns it.
async fn call_once(hosted: &HostedPlugin, tool: &str, call: Value) -> ToolCallView {
    let core = &hosted.core;
    core.model
        .push_reply(tool_calls(&[("call_1", tool, &arguments(call))]));
    core.model.push_reply(reply("Done."));
    let (conversation, run) = start(core, "OWNER: use the server").await;
    answer(core, &conversation, ApprovalChoice::AllowOnce).await;
    let transcript = core.wait_for_run(&conversation, &run).await;
    let [call] = transcript.tool_calls(0).try_into().expect("one call ran");
    call
}

#[tokio::test]
async fn a_local_servers_tool_runs_once_the_owner_allows_it() {
    let hosted = hosted().await;
    let server = connect(&hosted, local("Test", &[], 300)).await;
    let id = server["id"].as_str().unwrap().to_owned();
    let program = server_program().display().to_string();
    let core = &hosted.core;
    core.model.push_reply(tool_calls(&[(
        "call_1",
        "mcp_test_echo",
        &arguments(json!({ "text": "hello" })),
    )]));
    core.model.push_reply(reply("It said hello."));

    let (conversation, run) = start(core, "OWNER: say hello").await;
    let request = answer(core, &conversation, ApprovalChoice::AllowOnce).await;
    let transcript = core.wait_for_run(&conversation, &run).await;

    let endpoint = ToolServerEndpoint::Program {
        program,
        arguments: Vec::new(),
    };
    assert_eq!(server["protocolVersion"], "2026-07-28");
    assert_eq!(request.reasons, vec![ApprovalReason::ExternalTool]);
    assert_eq!(
        request.grants.first(),
        Some(&GrantScope::ExternalTool {
            plugin: "mcp".to_owned(),
            target: id.clone(),
            label: "Test".to_owned(),
            endpoint: endpoint.clone(),
            tool: "echo".to_owned(),
        })
    );
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(call.decision, Some(Decision::AllowedOnce));
    assert!(
        matches!(
            &call.accesses[..],
            [Access::ExternalTool { target, tool, arguments, endpoint: seen, .. }]
                if *target == id && tool == "echo" && arguments == r#"{"text":"hello"}"# && *seen == endpoint
        ),
        "{:?}",
        call.accesses
    );
    assert_eq!(output(&call), "Echo: hello");
}

#[tokio::test]
async fn a_grant_covers_one_tool_with_any_arguments_and_no_other_tool() {
    let hosted = hosted().await;
    connect(&hosted, local("Test", &[], 300)).await;
    let core = &hosted.core;
    core.model.push_reply(tool_calls(&[(
        "call_1",
        "mcp_test_echo",
        &arguments(json!({ "text": "one" })),
    )]));
    core.model.push_reply(tool_calls(&[(
        "call_2",
        "mcp_test_echo",
        &arguments(json!({ "text": "two" })),
    )]));
    core.model.push_reply(tool_calls(&[(
        "call_3",
        "mcp_test_add",
        &arguments(json!({ "a": 1, "b": 2 })),
    )]));
    core.model.push_reply(reply("Done."));

    let (conversation, run) = start(core, "OWNER: echo twice, then add").await;
    answer(core, &conversation, ApprovalChoice::AllowForConversation).await;
    let other = answer(core, &conversation, ApprovalChoice::AllowOnce).await;
    let transcript = core.wait_for_run(&conversation, &run).await;

    let calls = transcript.tool_calls(0);
    let decisions: Vec<Option<Decision>> = calls.iter().map(|call| call.decision).collect();
    assert_eq!(
        decisions,
        [
            Some(Decision::AllowedForConversation),
            Some(Decision::Grant),
            Some(Decision::AllowedOnce)
        ]
    );
    assert!(matches!(
        other.grants.first(),
        Some(GrantScope::ExternalTool { tool, .. }) if tool == "add"
    ));
    assert_eq!(output(&calls[1]), "Echo: two");
    assert_eq!(output(&calls[2]), "3");
}

#[tokio::test]
async fn the_servers_secret_reaches_its_process_and_never_the_result_or_the_log() {
    let hosted = hosted().await;
    let server = connect(&hosted, local("Test", &[], 300)).await;
    let id = server["id"].as_str().unwrap().to_owned();

    let call = call_once(
        &hosted,
        "mcp_test_read_variable",
        json!({ "name": SECRET_VARIABLE }),
    )
    .await;
    // The server repeats the secret in an error, its instructions and a
    // tool's description, as a careless server does.
    let failed = call_once(&hosted, "mcp_test_fail", json!({ "leak": SECRET_VARIABLE })).await;
    let log = hosted
        .invoke("server_log", json!({ "serverId": id }))
        .await
        .unwrap();
    let tools = hosted
        .invoke("server_tools", json!({ "serverId": id }))
        .await
        .unwrap();

    assert_eq!(output(&call), "[REDACTED]");
    assert!(
        output(&failed).contains("the call failed with the token [REDACTED]"),
        "{}",
        output(&failed)
    );
    let log = log.to_string();
    assert!(log.contains("starting with the token [REDACTED]"), "{log}");
    assert!(!log.contains(PLANTED));
    let tools = tools.to_string();
    assert!(tools.contains("Your token is [REDACTED]."), "{tools}");
    assert!(!tools.contains(PLANTED));
    let requests = format!("{:?}", hosted.core.model.requests());
    assert!(requests.contains("[REDACTED]"));
    assert!(!requests.contains(PLANTED));
}

#[tokio::test]
async fn an_image_of_a_result_reaches_the_model_and_the_call() {
    let hosted = hosted().await;
    connect(&hosted, local("Test", &[], 300)).await;

    let call = call_once(&hosted, "mcp_test_show_image", json!({})).await;

    let result = call.result.as_ref().unwrap();
    assert_eq!(result.images.len(), 1, "{result:?}");
    assert_eq!(result.output, "One pixel:");
}

#[tokio::test]
async fn an_activity_argument_is_renamed_for_the_model_and_named_back_for_the_server() {
    let hosted = hosted().await;
    let server = connect(&hosted, local("Test", &[], 300)).await;
    let id = server["id"].as_str().unwrap().to_owned();
    let tools = hosted
        .invoke("server_tools", json!({ "serverId": id }))
        .await
        .unwrap();

    let call = call_once(
        &hosted,
        "mcp_test_track_activity",
        json!({ "activity_value": "hiking" }),
    )
    .await;

    let tracked = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "track-activity")
        .unwrap();
    assert_eq!(tracked["modelName"], "mcp_test_track_activity");
    assert_eq!(output(&call), "Activity: hiking");
}

#[tokio::test]
async fn a_tool_that_the_server_adds_later_starts_off_and_marked_as_new() {
    let hosted = hosted().await;
    let server = connect(&hosted, local("Test", &[], 300)).await;
    let id = server["id"].as_str().unwrap().to_owned();

    call_once(&hosted, "mcp_test_add_tool", json!({})).await;
    let later = wait_for_view(&hosted, &id, |server| {
        server["warnings"].as_array().is_some_and(|warnings| {
            warnings
                .iter()
                .any(|warning| warning["code"] == "new_tools")
        })
    })
    .await;
    let tools = hosted
        .invoke("server_tools", json!({ "serverId": id }))
        .await
        .unwrap();

    let late = tools
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == LATE_TOOL)
        .cloned()
        .unwrap();
    assert_eq!(late["enabled"], false);
    assert_eq!(late["new"], true);
    assert_eq!(
        later["toolCount"],
        server["toolCount"].as_u64().unwrap() + 1
    );
    assert!(!published_tools(&hosted.core.runtime).contains(&"mcp_test_late_tool".to_owned()));
}

#[tokio::test]
async fn a_crash_during_a_call_leaves_its_outcome_unknown_and_the_server_starts_again() {
    let hosted = hosted().await;
    let server = connect(&hosted, local("Test", &[], 300)).await;
    let id = server["id"].as_str().unwrap().to_owned();

    let call = call_once(&hosted, "mcp_test_crash", json!({})).await;
    let restarted = wait_for_view(&hosted, &id, |server| server["status"] == "connected").await;
    let log = hosted
        .invoke("server_log", json!({ "serverId": id }))
        .await
        .unwrap();

    let result = call.result.as_ref().unwrap();
    assert!(
        output(&call).contains("whether the call took effect is unknown"),
        "{}",
        output(&call)
    );
    assert!(
        format!("{:?}", result.outcome).contains("server_stopped"),
        "{result:?}"
    );
    assert_eq!(restarted["status"], "connected");
    assert!(log.to_string().contains("restarting"), "{log}");
}

#[tokio::test]
async fn a_call_past_the_servers_timeout_ends_and_leaves_its_outcome_unknown() {
    let hosted = hosted().await;
    connect(&hosted, local("Test", &[], 10)).await;

    let call = call_once(&hosted, "mcp_test_sleep", json!({ "seconds": 60 })).await;

    assert!(
        output(&call).contains("did not answer within 10 seconds"),
        "{}",
        output(&call)
    );
}

#[tokio::test]
async fn a_resource_is_read_with_mcp_read_resource_under_an_approval() {
    let hosted = hosted().await;
    connect(&hosted, local("Test", &[], 300)).await;
    tokio::time::timeout(Duration::from_secs(20), async {
        while !published_tools(&hosted.core.runtime).contains(&"mcp_read_resource".to_owned()) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the read tool is published");

    let call = call_once(
        &hosted,
        "mcp_read_resource",
        json!({ "server": "test", "uri": "test://readme" }),
    )
    .await;

    assert!(
        matches!(&call.accesses[..], [Access::ExternalTool { tool, arguments, .. }] if tool == "read_resource" && arguments == "test://readme"),
        "{:?}",
        call.accesses
    );
    assert!(output(&call).contains(README_TEXT), "{}", output(&call));
}

#[tokio::test]
async fn a_server_of_2025_connects_with_the_initialize_handshake() {
    let hosted = hosted().await;

    let server = connect(&hosted, local("Old", &["--legacy"], 300)).await;
    let call = call_once(&hosted, "mcp_old_echo", json!({ "text": "hi" })).await;

    assert_eq!(server["protocolVersion"], legacy::VERSION);
    assert_eq!(output(&call), "Echo: hi");
}

#[tokio::test]
async fn a_remote_server_takes_its_secret_header_and_refuses_a_wrong_one() {
    let hosted = hosted().await;
    let remote = serve_http(HttpOptions {
        authorization: Some(format!("Bearer {PLANTED}")),
        ..HttpOptions::default()
    })
    .await
    .unwrap();
    let server = |name: &str, token: &str| {
        json!({
            "name": name,
            "transport": "http",
            "url": remote.url(),
            "secrets": [{ "name": "Authorization", "value": format!("Bearer {token}") }],
            "images": true,
            "instructions": true,
            "timeoutSeconds": 300,
        })
    };

    connect(&hosted, server("Remote", PLANTED)).await;
    let call = call_once(&hosted, "mcp_remote_echo", json!({ "text": "over HTTP" })).await;
    let wrong = hosted
        .invoke("add_server", server("Wrong", "wrong-token-value"))
        .await
        .unwrap();
    let wrong_id = wrong["id"].as_str().unwrap().to_owned();
    let failed = wait_for_view(&hosted, &wrong_id, |server| server["status"] == "failed").await;

    assert_eq!(output(&call), "Echo: over HTTP");
    assert_eq!(
        failed["failure"],
        json!({ "code": "http_status", "status": 401 })
    );
}

#[tokio::test]
async fn turning_the_plugin_off_ends_its_servers_and_takes_their_tools() {
    let hosted = hosted().await;
    connect(&hosted, local("Test", &[], 300)).await;

    hosted.turn_off().await;

    assert!(
        !published_tools(&hosted.core.runtime)
            .iter()
            .any(|tool| tool.starts_with("mcp_"))
    );
}

#[tokio::test]
async fn a_mention_lists_a_servers_resource_and_attaches_its_text_to_the_message() {
    let hosted = hosted().await;
    let server = connect(&hosted, local("Test", &[], 300)).await;
    let id = server["id"].as_str().unwrap().to_owned();
    let core = &hosted.core;
    let place = MentionPlace::NewConversation { project_id: None };

    let suggestions = core
        .runtime
        .suggest_mentions(SuggestMentions::new(place, "read"))
        .await
        .expect("the providers suggest");
    let group = suggestions
        .groups
        .iter()
        .find(|group| group.provider == "mcp")
        .expect("the plugin suggests resources");
    let entry = group.entries.first().expect("the readme is suggested");
    core.model.push_reply(reply("Read it."));
    let mut message = StartConversation::new("OWNER: what does this say?");
    message.mentions = vec![MentionRef {
        provider: "mcp".to_owned(),
        key: entry.key.clone(),
    }];
    let started = core
        .runtime
        .start_conversation(message)
        .await
        .expect("the conversation starts");
    core.wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    assert_eq!(entry.key, format!("{id}:test://readme"));
    assert_eq!(entry.label, "Read me");
    let requests = format!("{:?}", core.model.requests());
    assert!(requests.contains(README_TEXT), "{requests}");
}

/// A remote server that signs in with OAuth.
fn signing_in(url: &str) -> Value {
    json!({
        "name": "Remote",
        "transport": "http",
        "url": url,
        "oauth": true,
        "images": true,
        "instructions": true,
        "timeoutSeconds": 300,
    })
}

/// The page that the browser shows when a sign-in returns.
fn page() -> Value {
    json!({
        "title": "Mannie sign-in",
        "signedIn": "You are signed in.",
        "failed": "The sign-in failed.",
    })
}

#[tokio::test]
async fn a_sign_in_registers_mannie_redeems_the_code_with_pkce_and_connects() {
    let secrets = Arc::new(FakeSecrets::new());
    let hosted = hosted_with(&secrets).await;
    let remote = serve_http(HttpOptions {
        oauth: Some(OAuthOptions::default()),
        ..HttpOptions::default()
    })
    .await
    .unwrap();
    let added = hosted
        .invoke("add_server", signing_in(&remote.url()))
        .await
        .unwrap();
    let id = added["id"].as_str().unwrap().to_owned();
    wait_for_view(&hosted, &id, |server| server["status"] == "sign_in_needed").await;

    let started = hosted
        .invoke("begin_sign_in", json!({ "serverId": id, "page": page() }))
        .await
        .unwrap();
    let returned = sign_in_through(started["url"].as_str().unwrap())
        .await
        .unwrap();
    let connected = wait_for_view(&hosted, &id, |server| server["status"] == "connected").await;
    let call = call_once(&hosted, "mcp_remote_echo", json!({ "text": "signed in" })).await;

    assert_eq!(started["host"], "127.0.0.1");
    assert!(
        returned.body.contains("You are signed in."),
        "{}",
        returned.body
    );
    assert_eq!(connected["signedIn"], true);
    assert_eq!(output(&call), "Echo: signed in");
    let [token] = remote.access_tokens().try_into().unwrap();
    let stored = secrets
        .stored(&format!("mcp/oauth/{id}-0"))
        .expect("the tokens are stored");
    assert!(stored.expose().contains(&token));
    let log = hosted
        .invoke("server_log", json!({ "serverId": id }))
        .await
        .unwrap();
    assert!(!connected.to_string().contains(&token));
    assert!(!log.to_string().contains(&token));
    let signed_out = hosted
        .invoke("sign_out", json!({ "serverId": id }))
        .await
        .unwrap();
    assert_eq!(signed_out["signedIn"], false);
    wait_for_view(&hosted, &id, |server| server["status"] == "sign_in_needed").await;
}

#[tokio::test]
async fn a_sign_in_is_refused_by_an_authorization_server_without_s256() {
    let hosted = hosted().await;
    let remote = serve_http(HttpOptions {
        oauth: Some(OAuthOptions {
            s256: false,
            ..OAuthOptions::default()
        }),
        ..HttpOptions::default()
    })
    .await
    .unwrap();
    let added = hosted
        .invoke("add_server", signing_in(&remote.url()))
        .await
        .unwrap();
    let id = added["id"].as_str().unwrap().to_owned();

    let refused = hosted
        .invoke("begin_sign_in", json!({ "serverId": id, "page": page() }))
        .await
        .unwrap_err();

    assert_eq!(refused.code(), "mcp.authorization_unsupported");
}

#[tokio::test]
async fn an_expired_token_is_refreshed_without_a_new_sign_in() {
    let secrets = Arc::new(FakeSecrets::new());
    let hosted = hosted_with(&secrets).await;
    let remote = serve_http(HttpOptions {
        oauth: Some(OAuthOptions {
            expires_in: 1,
            ..OAuthOptions::default()
        }),
        ..HttpOptions::default()
    })
    .await
    .unwrap();
    let added = hosted
        .invoke("add_server", signing_in(&remote.url()))
        .await
        .unwrap();
    let id = added["id"].as_str().unwrap().to_owned();
    wait_for_view(&hosted, &id, |server| server["status"] == "sign_in_needed").await;
    let started = hosted
        .invoke("begin_sign_in", json!({ "serverId": id, "page": page() }))
        .await
        .unwrap();
    sign_in_through(started["url"].as_str().unwrap())
        .await
        .unwrap();
    wait_for_view(&hosted, &id, |server| server["status"] == "connected").await;
    tokio::time::sleep(Duration::from_secs(2)).await;

    let call = call_once(&hosted, "mcp_remote_echo", json!({ "text": "later" })).await;

    assert_eq!(output(&call), "Echo: later");
    let tokens = remote.access_tokens();
    assert!(tokens.len() >= 2, "{tokens:?}");
    let latest = tokens.last().unwrap();
    let stored = secrets.stored(&format!("mcp/oauth/{id}-0")).unwrap();
    assert!(stored.expose().contains(latest.as_str()));
}
