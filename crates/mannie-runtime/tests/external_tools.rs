//! Calls of a tool server's tools through a plugin's tool (ADR-0043,
//! decision 4): the policy asks with the server, its endpoint and the tool,
//! a grant covers the same tool of the same server at the same endpoint with
//! any arguments and no rule is offered, full access runs every call, a
//! server whose endpoint changed after the approval is asked about again, and
//! the call's view cuts long arguments while the tool executes them whole.

#![cfg(test)]

use std::sync::{Arc, Mutex};

use futures_util::FutureExt;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::points::tools::{
    AgentSelector, ExternalToolAccess, PluginTool, Tool, ToolAccess, ToolCall, ToolExecution,
    ToolRefusal, ToolResult, ToolSpec, Tools,
};
use mannie_runtime::testing::{
    Harness, HarnessOptions, SeenPart, Transcript, add_plugin, reply, tool_calls,
};
use mannie_types::chat::StartConversation;
use mannie_types::ids::{ConversationId, PartId, RunId};
use mannie_types::run::RunState;
use mannie_types::tools::{
    Access, ApprovalChoice, ApprovalReason, ApprovalRequest, DecideApproval, Decision, GrantScope,
    MAX_VIEW_ARGUMENTS_BYTES, ToolCallStatus, ToolHint, ToolServerEndpoint, UpdateToolsSettings,
};
use mannie_types::transcript::PartContent;
use serde_json::{Value, json};

/// The fake plugin's ID.
const PLUGIN: &str = "servers";

/// The fake plugin's tool.
const CALL_TOOL: &str = "call_tool";

/// Where the fake plugin's servers run, until a test moves one.
const URL: &str = "https://mcp.example.com/mcp";

/// The plugin's key for a server, by its name.
fn target_of(server: &str) -> String {
    format!("key-of-{server}")
}

fn url(url: &str) -> ToolServerEndpoint {
    ToolServerEndpoint::Url {
        url: url.to_owned(),
    }
}

/// A plugin's tool that calls a tool of one of the plugin's tool servers.
/// Its preparation names the server's current endpoint, which a test may
/// move, and it keeps what it executed.
#[derive(Debug)]
struct ServerTools {
    endpoint: Mutex<ToolServerEndpoint>,
    executions: Mutex<Vec<ToolExecution>>,
}

impl ServerTools {
    fn new() -> Self {
        Self {
            endpoint: Mutex::new(url(URL)),
            executions: Mutex::new(Vec::new()),
        }
    }

    fn move_to(&self, endpoint: ToolServerEndpoint) {
        *self.endpoint.lock().unwrap() = endpoint;
    }

    fn executions(&self) -> Vec<ToolExecution> {
        self.executions.lock().unwrap().clone()
    }
}

impl Tool for ServerTools {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            CALL_TOOL,
            "Calls a tool of a tool server.",
            json!({
                "additionalProperties": false,
                "properties": {
                    "server": { "type": "string" },
                    "tool": { "type": "string" },
                    "payload": { "type": "string" },
                },
                "required": ["server", "tool", "payload"],
                "type": "object",
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let endpoint = self.endpoint.lock().unwrap().clone();
        async move {
            let arguments: Value = serde_json::from_str(&call.arguments)
                .map_err(|error| ToolRefusal::new(error.to_string()))?;
            let text = |key: &str| arguments[key].as_str().unwrap_or_default().to_owned();
            let server = text("server");
            let access = ExternalToolAccess::new(
                target_of(&server),
                server,
                endpoint,
                text("tool"),
                text("payload"),
            )
            .with_title("Create an issue")
            .with_hints(vec![ToolHint::OpenWorld]);
            Ok(vec![ToolAccess::ExternalTool(access)])
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let called: Vec<String> = execution
            .authorized
            .iter()
            .filter_map(|access| match access {
                ToolAccess::ExternalTool(external) => Some(external.tool.clone()),
                ToolAccess::Path(_) | ToolAccess::RemoteCommand(_) | ToolAccess::Program(_) => None,
            })
            .collect();
        self.executions.lock().unwrap().push(execution);
        async move { ToolResult::new(format!("Called {}.", called.join(", "))) }.boxed()
    }
}

/// The fake plugin with its tool, which has side effects and is offered to
/// the owner's conversations.
fn with_servers(servers: &Arc<ServerTools>) -> Contributions {
    let tool: Arc<dyn Tool> = Arc::<ServerTools>::clone(servers);
    Contributions::new()
        .add::<Tools>(PluginTool::new(tool, AgentSelector::Conversations).with_side_effects())
}

/// The arguments of a call of the fake tool.
fn calling(server: &str, tool: &str, payload: &str) -> String {
    json!({ "activity": "Calling", "server": server, "tool": tool, "payload": payload }).to_string()
}

/// The access that a call of the fake tool hands the core.
fn access(server: &str, endpoint: ToolServerEndpoint, tool: &str, payload: &str) -> Access {
    Access::ExternalTool {
        plugin: PLUGIN.to_owned(),
        target: target_of(server),
        label: server.to_owned(),
        endpoint,
        tool: tool.to_owned(),
        title: Some("Create an issue".to_owned()),
        arguments: payload.to_owned(),
        arguments_cut: false,
        hints: vec![ToolHint::OpenWorld],
    }
}

fn grant(server: &str, endpoint: ToolServerEndpoint, tool: &str) -> GrantScope {
    GrantScope::ExternalTool {
        plugin: PLUGIN.to_owned(),
        target: target_of(server),
        label: server.to_owned(),
        endpoint,
        tool: tool.to_owned(),
    }
}

async fn harness() -> Harness {
    Harness::with_options(HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    })
    .await
}

async fn start(harness: &Harness, text: &str) -> (ConversationId, RunId) {
    let started = harness
        .runtime
        .start_conversation(StartConversation::new(text))
        .await
        .unwrap();
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

/// Waits until a call waits for the owner and returns its request.
async fn request_of(harness: &Harness, conversation: &ConversationId) -> (PartId, ApprovalRequest) {
    let transcript = harness
        .wait_until(conversation, |transcript| {
            waiting_call(transcript).is_some()
        })
        .await;
    waiting_call(&transcript).unwrap()
}

/// Answers the request of `part_id` with `choice`, and waits until that
/// request no longer waits, so the next answer finds the next request.
async fn decide(
    harness: &Harness,
    conversation: &ConversationId,
    (part_id, request): (PartId, ApprovalRequest),
    choice: ApprovalChoice,
) {
    harness
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation.clone(),
            part_id: part_id.clone(),
            digest: request.digest.clone(),
            choice,
        })
        .await
        .unwrap();
    harness
        .wait_until(conversation, |transcript| {
            waiting_call(transcript).is_none_or(|waiting| waiting.1.digest != request.digest)
        })
        .await;
}

/// Waits until a call waits for the owner and answers it with `choice`.
async fn answer(harness: &Harness, conversation: &ConversationId, choice: ApprovalChoice) {
    let waiting = request_of(harness, conversation).await;
    decide(harness, conversation, waiting, choice).await;
}

#[tokio::test]
async fn a_tool_servers_call_asks_with_its_tool_and_runs_what_was_authorized_once_allowed() {
    let harness = harness().await;
    let servers = Arc::new(ServerTools::new());
    add_plugin(&harness.runtime, PLUGIN, with_servers(&servers)).unwrap();
    let payload = r#"{"title":"Crash on start"}"#;
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        CALL_TOOL,
        &calling("issues", "create_issue", payload),
    )]));
    harness.model.push_reply(reply("Filed."));

    let (conversation, run) = start(&harness, "OWNER: file the crash").await;
    let (part, request) = request_of(&harness, &conversation).await;
    decide(
        &harness,
        &conversation,
        (part, request.clone()),
        ApprovalChoice::AllowOnce,
    )
    .await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    assert_eq!(request.reasons, vec![ApprovalReason::ExternalTool]);
    assert_eq!(
        request.grants,
        vec![grant("issues", url(URL), "create_issue")]
    );
    assert_eq!(request.rule, None);
    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(call.decision, Some(Decision::AllowedOnce));
    assert_eq!(
        call.accesses,
        vec![access("issues", url(URL), "create_issue", payload)]
    );
    assert_eq!(
        call.result.map(|result| result.output),
        Some("Called create_issue.".to_owned())
    );
    let [execution] = servers.executions().try_into().unwrap();
    let [ToolAccess::ExternalTool(authorized)] = &execution.authorized[..] else {
        panic!("not a tool server's call: {:?}", execution.authorized);
    };
    assert_eq!(authorized.arguments, payload);
    assert_eq!(authorized.endpoint, url(URL));
}

#[tokio::test]
async fn a_grant_for_the_conversation_covers_the_same_tool_of_the_same_server_with_any_arguments() {
    let harness = harness().await;
    let servers = Arc::new(ServerTools::new());
    add_plugin(&harness.runtime, PLUGIN, with_servers(&servers)).unwrap();
    for (id, server, tool, payload) in [
        ("call_1", "issues", "create_issue", r#"{"title":"A"}"#),
        ("call_2", "issues", "create_issue", r#"{"title":"B"}"#),
        ("call_3", "issues", "delete_issue", r#"{"number":1}"#),
        ("call_4", "tracker", "create_issue", r#"{"title":"A"}"#),
    ] {
        harness.model.push_reply(tool_calls(&[(
            id,
            CALL_TOOL,
            &calling(server, tool, payload),
        )]));
    }
    harness.model.push_reply(reply("Done."));

    let (conversation, run) = start(&harness, "OWNER: file both").await;
    answer(
        &harness,
        &conversation,
        ApprovalChoice::AllowForConversation,
    )
    .await;
    answer(&harness, &conversation, ApprovalChoice::Deny).await;
    answer(&harness, &conversation, ApprovalChoice::Deny).await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let decisions: Vec<_> = transcript
        .tool_calls(0)
        .iter()
        .map(|call| call.decision)
        .collect();
    assert_eq!(
        decisions,
        vec![
            Some(Decision::AllowedForConversation),
            Some(Decision::Grant),
            Some(Decision::Denied),
            Some(Decision::Denied),
        ]
    );
    assert_eq!(servers.executions().len(), 2);
}

#[tokio::test]
async fn a_grant_for_every_conversation_covers_the_tool_elsewhere_and_no_new_server_at_its_endpoint()
 {
    let harness = harness().await;
    let servers = Arc::new(ServerTools::new());
    add_plugin(&harness.runtime, PLUGIN, with_servers(&servers)).unwrap();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        CALL_TOOL,
        &calling("issues", "create_issue", "{}"),
    )]));
    harness.model.push_reply(reply("Filed."));
    harness.model.push_reply(tool_calls(&[(
        "call_2",
        CALL_TOOL,
        &calling("issues", "create_issue", r#"{"title":"B"}"#),
    )]));
    harness.model.push_reply(tool_calls(&[(
        "call_3",
        CALL_TOOL,
        &calling("issues-again", "create_issue", r#"{"title":"B"}"#),
    )]));
    harness.model.push_reply(reply("Done."));

    let (first, first_run) = start(&harness, "OWNER: file one").await;
    answer(&harness, &first, ApprovalChoice::AllowAlways).await;
    harness.wait_for_run(&first, &first_run).await;
    let (second, second_run) = start(&harness, "OWNER: file another").await;
    // A server that the owner registered again at the same endpoint has
    // another key, so the grant does not cover it.
    answer(&harness, &second, ApprovalChoice::Deny).await;
    let transcript = harness.wait_for_run(&second, &second_run).await;

    let decisions: Vec<_> = transcript
        .tool_calls(0)
        .iter()
        .map(|call| call.decision)
        .collect();
    assert_eq!(
        decisions,
        vec![Some(Decision::AlwaysGrant), Some(Decision::Denied)]
    );
    let [shared] = harness
        .runtime
        .shared_grants()
        .await
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(shared.scope, grant("issues", url(URL), "create_issue"));
    assert_eq!(shared.project_id, None);
}

#[tokio::test]
async fn full_access_runs_a_tool_servers_call_without_asking() {
    let harness = harness().await;
    let servers = Arc::new(ServerTools::new());
    add_plugin(&harness.runtime, PLUGIN, with_servers(&servers)).unwrap();
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
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        CALL_TOOL,
        &calling("issues", "delete_repository", r#"{"name":"app"}"#),
    )]));
    harness.model.push_reply(reply("Deleted."));

    let (conversation, run) = start(&harness, "OWNER: delete it").await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.decision, Some(Decision::FullAccess));
    assert_eq!(call.status, ToolCallStatus::Completed);
}

#[tokio::test]
async fn a_server_that_moved_after_the_approval_is_asked_about_again() {
    let harness = harness().await;
    let servers = Arc::new(ServerTools::new());
    add_plugin(&harness.runtime, PLUGIN, with_servers(&servers)).unwrap();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        CALL_TOOL,
        &calling("issues", "create_issue", "{}"),
    )]));
    harness.model.push_reply(reply("Filed."));
    let moved = url("https://mcp.example.net/mcp");

    let (conversation, run) = start(&harness, "OWNER: file it").await;
    let first = request_of(&harness, &conversation).await;
    servers.move_to(moved.clone());
    decide(
        &harness,
        &conversation,
        first.clone(),
        ApprovalChoice::AllowForConversation,
    )
    .await;
    let second = request_of(&harness, &conversation).await;
    decide(
        &harness,
        &conversation,
        second.clone(),
        ApprovalChoice::AllowOnce,
    )
    .await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    assert_eq!(first.0, second.0);
    assert_ne!(first.1.digest, second.1.digest);
    assert_eq!(
        second.1.grants,
        vec![grant("issues", moved.clone(), "create_issue")]
    );
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.decision, Some(Decision::AllowedOnce));
    let [execution] = servers.executions().try_into().unwrap();
    let [ToolAccess::ExternalTool(authorized)] = &execution.authorized[..] else {
        panic!("not a tool server's call: {:?}", execution.authorized);
    };
    assert_eq!(authorized.endpoint, moved);
}

#[tokio::test]
async fn the_view_cuts_long_arguments_of_a_tool_servers_call_and_the_tool_gets_them_whole() {
    let harness = harness().await;
    let servers = Arc::new(ServerTools::new());
    add_plugin(&harness.runtime, PLUGIN, with_servers(&servers)).unwrap();
    let payload = json!({ "body": "x".repeat(MAX_VIEW_ARGUMENTS_BYTES * 2) }).to_string();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        CALL_TOOL,
        &calling("issues", "create_issue", &payload),
    )]));
    harness.model.push_reply(reply("Filed."));

    let (conversation, run) = start(&harness, "OWNER: file the long one").await;
    answer(&harness, &conversation, ApprovalChoice::AllowOnce).await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    let [
        Access::ExternalTool {
            arguments,
            arguments_cut,
            ..
        },
    ] = &call.accesses[..]
    else {
        panic!("not a tool server's call: {:?}", call.accesses);
    };
    assert!(*arguments_cut);
    assert_eq!(arguments.len(), MAX_VIEW_ARGUMENTS_BYTES);
    assert!(payload.starts_with(arguments.as_str()));
    let [execution] = servers.executions().try_into().unwrap();
    let [ToolAccess::ExternalTool(authorized)] = &execution.authorized[..] else {
        panic!("not a tool server's call: {:?}", execution.authorized);
    };
    assert_eq!(authorized.arguments, payload);
}
