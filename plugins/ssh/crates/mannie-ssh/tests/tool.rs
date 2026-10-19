//! The tool `run_ssh_command` through the core's tool loop, with a scripted
//! model and an SSH server in the test's process (plugins/ssh/docs/
//! README.md, the tool; ADR-0036): the owner's approval, grants that cover
//! one host, the output with its archives and without the password, the
//! timeout and Stop, which ask the host to end the command, and the hosts
//! that the model reads.

use std::sync::Arc;
use std::time::Duration;

use mannie_plugin_harness::host::{HostOptions, HostedPlugin};
use mannie_plugin_harness::runtime::{
    Harness, HarnessOptions, SeenPart, Transcript, reply, tool_calls,
};
use mannie_plugin_harness::types::{
    Access, ApprovalChoice, ApprovalReason, ApprovalRequest, DecideApproval, Decision, GrantScope,
    Interruption, PartContent, PartId, ReadToolOutput, RunId, RunState, StartConversation,
    ToolCallStatus, ToolCallView,
};
use mannie_plugin_sdk::contracts::ConversationId;
use mannie_plugin_sdk::testing::FakeSecrets;
use mannie_ssh::{RUN_SSH_COMMAND, mannie_plugin};
use mannie_ssh_testing::{Options, PASSWORD, TestServer, USERNAME};
use serde_json::{Value, json};

/// The plugin, on, in a runtime whose conversations use tools.
async fn hosted() -> HostedPlugin {
    let core = HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    };
    let options = HostOptions::new()
        .with_core(core)
        .with_provider(Arc::new(FakeSecrets::new()));
    let hosted = HostedPlugin::new(mannie_plugin(), options).await;
    hosted.turn_on().await;
    hosted
}

/// Registers a host of `server` and trusts its key, as the owner does in
/// the settings. Returns the host's ID.
async fn register(hosted: &HostedPlugin, name: &str, server: &TestServer) -> String {
    let added = hosted
        .invoke(
            "add_host",
            json!({
                "name": name,
                "address": "127.0.0.1",
                "port": server.port(),
                "username": USERNAME,
                "password": PASSWORD,
                "note": "The test server.",
            }),
        )
        .await
        .expect("the host is added");
    let id = added["id"].as_str().expect("a host has an ID").to_owned();
    hosted
        .invoke(
            "trust_host_key",
            json!({ "hostId": id, "fingerprint": scanned(hosted, &id).await }),
        )
        .await
        .expect("the scanned key is trusted");
    id
}

async fn scanned(hosted: &HostedPlugin, id: &str) -> Value {
    let scan = hosted
        .invoke("scan_host_key", json!({ "hostId": id }))
        .await
        .expect("the host's key is scanned");
    scan["key"]["fingerprint"].clone()
}

/// The arguments of a call on `host`.
fn on(host: &str, command: &str, timeout_seconds: Option<u32>) -> String {
    json!({
        "activity": "Running",
        "host": host,
        "command": command,
        "timeout_seconds": timeout_seconds,
    })
    .to_string()
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

/// Waits until the server saw `signal`, which a task of the plugin sends
/// after the call's future was dropped.
async fn wait_for_signal(server: &TestServer, signal: &str) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !server.signals().iter().any(|seen| seen == signal) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the signal reaches the host");
}

#[tokio::test]
async fn a_command_runs_on_the_host_once_the_owner_allows_it() {
    let hosted = hosted().await;
    let server = TestServer::start(Options::default()).await.unwrap();
    let id = register(&hosted, "build-box", &server).await;
    let core = &hosted.core;
    core.model.push_reply(tool_calls(&[(
        "call_1",
        RUN_SSH_COMMAND,
        &on("build-box", "echo hello", None),
    )]));
    core.model.push_reply(reply("It said hello."));

    let (conversation, run) = start(core, "OWNER: say hello on build-box").await;
    let request = answer(core, &conversation, ApprovalChoice::AllowOnce).await;
    let transcript = core.wait_for_run(&conversation, &run).await;

    let endpoint = format!("{USERNAME}@127.0.0.1:{}", server.port());
    assert_eq!(request.reasons, vec![ApprovalReason::RemoteCommand]);
    assert_eq!(
        request.grants,
        vec![GrantScope::RemoteCommand {
            plugin: "ssh".to_owned(),
            target: id.clone(),
            label: "build-box".to_owned(),
            endpoint: endpoint.clone(),
            command: "echo hello".to_owned(),
        }]
    );
    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(call.decision, Some(Decision::AllowedOnce));
    assert_eq!(
        call.accesses,
        vec![Access::RemoteCommand {
            plugin: "ssh".to_owned(),
            target: id,
            label: "build-box".to_owned(),
            endpoint: endpoint.clone(),
            identity: server.fingerprint(),
            command: "echo hello".to_owned(),
        }]
    );
    assert!(
        output(&call).starts_with(&format!("Ran on build-box ({endpoint}).\nExit code: 0\n")),
        "{}",
        output(&call)
    );
    assert!(output(&call).contains("Standard output (6 bytes):\nhello\n"));
    assert_eq!(server.commands(), ["echo hello"]);
}

#[tokio::test]
async fn a_grant_for_the_conversation_runs_the_command_again_on_its_host_only() {
    let hosted = hosted().await;
    let first = TestServer::start(Options::default()).await.unwrap();
    let second = TestServer::start(Options::default()).await.unwrap();
    register(&hosted, "first", &first).await;
    register(&hosted, "second", &second).await;
    let core = &hosted.core;
    core.model.push_reply(tool_calls(&[(
        "call_1",
        RUN_SSH_COMMAND,
        &on("first", "uptime", None),
    )]));
    core.model.push_reply(tool_calls(&[(
        "call_2",
        RUN_SSH_COMMAND,
        &on("first", "uptime", None),
    )]));
    core.model.push_reply(tool_calls(&[(
        "call_3",
        RUN_SSH_COMMAND,
        &on("second", "uptime", None),
    )]));
    core.model.push_reply(reply("Done."));

    let (conversation, run) = start(core, "OWNER: check both").await;
    answer(core, &conversation, ApprovalChoice::AllowForConversation).await;
    let other = answer(core, &conversation, ApprovalChoice::Deny).await;
    let transcript = core.wait_for_run(&conversation, &run).await;

    let calls = transcript.tool_calls(0);
    let decisions: Vec<Option<Decision>> = calls.iter().map(|call| call.decision).collect();
    assert_eq!(
        decisions,
        [
            Some(Decision::AllowedForConversation),
            Some(Decision::Grant),
            Some(Decision::Denied)
        ]
    );
    assert!(matches!(
        &other.grants[..],
        [GrantScope::RemoteCommand { label, .. }] if label == "second"
    ));
    assert_eq!(first.commands(), ["uptime", "uptime"]);
    assert_eq!(second.commands(), Vec::<String>::new());
}

#[tokio::test]
async fn a_timeout_asks_the_host_to_end_the_command_and_says_that_processes_may_remain() {
    let hosted = hosted().await;
    let server = TestServer::start(Options::default()).await.unwrap();
    register(&hosted, "build-box", &server).await;
    let core = &hosted.core;
    core.model.push_reply(tool_calls(&[(
        "call_1",
        RUN_SSH_COMMAND,
        &on("build-box", "sleep", Some(1)),
    )]));
    core.model.push_reply(reply("It timed out."));

    let (conversation, run) = start(core, "OWNER: wait on build-box").await;
    answer(core, &conversation, ApprovalChoice::AllowOnce).await;
    let transcript = core.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert!(
        output(&call).contains("Timed out after 1 s"),
        "{}",
        output(&call)
    );
    assert!(output(&call).contains("may still run on the host"));
    assert_eq!(server.signals().first().map(String::as_str), Some("TERM"));
}

#[tokio::test]
async fn the_password_never_shows_in_the_output() {
    let hosted = hosted().await;
    let server = TestServer::start(Options::default()).await.unwrap();
    register(&hosted, "build-box", &server).await;
    let core = &hosted.core;
    core.model.push_reply(tool_calls(&[(
        "call_1",
        RUN_SSH_COMMAND,
        &on("build-box", "print-password", None),
    )]));
    core.model.push_reply(reply("Done."));

    let (conversation, run) = start(core, "OWNER: print it").await;
    answer(core, &conversation, ApprovalChoice::AllowOnce).await;
    let transcript = core.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert!(
        output(&call).contains("the password is [REDACTED]"),
        "{}",
        output(&call)
    );
    assert!(!output(&call).contains(PASSWORD));
}

#[tokio::test]
async fn output_that_does_not_fit_is_archived_for_read_tool_output() {
    let hosted = hosted().await;
    let server = TestServer::start(Options::default()).await.unwrap();
    register(&hosted, "build-box", &server).await;
    let core = &hosted.core;
    core.model.push_reply(tool_calls(&[(
        "call_1",
        RUN_SSH_COMMAND,
        &on("build-box", "lines 20000", None),
    )]));
    core.model.push_reply(reply("Done."));

    let (conversation, run) = start(core, "OWNER: list").await;
    answer(core, &conversation, ApprovalChoice::AllowOnce).await;
    let transcript = core.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    let result = call.result.as_ref().unwrap();
    let [archive] = result.archives.clone().try_into().unwrap();
    assert_eq!(archive.name, "stdout");
    assert!(result.output.contains(&format!("output_id {}", archive.id)));
    assert!(result.output.contains("line 1\n"));
    assert!(result.output.contains("line 20000\n"));
    let page = core
        .runtime
        .tool_output(ReadToolOutput {
            conversation_id: conversation.clone(),
            archive_id: archive.id,
            first_line: 10_000,
            limit: 2,
        })
        .await
        .unwrap();
    assert_eq!(page.lines, ["line 10000", "line 10001"]);
}

#[tokio::test]
async fn stopping_the_reply_asks_the_host_to_end_the_command_and_leaves_it_uncertain() {
    let hosted = hosted().await;
    let server = TestServer::start(Options::default()).await.unwrap();
    register(&hosted, "build-box", &server).await;
    let core = &hosted.core;
    core.model.push_reply(tool_calls(&[(
        "call_1",
        RUN_SSH_COMMAND,
        &on("build-box", "sleep", Some(600)),
    )]));

    let (conversation, run) = start(core, "OWNER: wait on build-box").await;
    answer(core, &conversation, ApprovalChoice::AllowOnce).await;
    tokio::time::timeout(Duration::from_secs(20), async {
        while server.commands().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the command reaches the host");
    core.runtime.stop_reply(&conversation).await;
    let transcript = core.wait_for_run(&conversation, &run).await;
    wait_for_signal(&server, "TERM").await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Stopped));
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Uncertain);
    assert_eq!(call.interruption, Some(Interruption::Stopped));
}

#[tokio::test]
async fn a_host_without_a_trusted_key_is_refused_before_anyone_is_asked() {
    let hosted = hosted().await;
    let server = TestServer::start(Options::default()).await.unwrap();
    hosted
        .invoke(
            "add_host",
            json!({
                "name": "new-box",
                "address": "127.0.0.1",
                "port": server.port(),
                "username": USERNAME,
                "password": PASSWORD,
            }),
        )
        .await
        .unwrap();
    let core = &hosted.core;
    core.model.push_reply(tool_calls(&[
        ("call_1", RUN_SSH_COMMAND, &on("new-box", "uptime", None)),
        ("call_2", RUN_SSH_COMMAND, &on("nowhere", "uptime", None)),
    ]));
    core.model.push_reply(reply("Neither ran."));

    let (conversation, run) = start(core, "OWNER: try").await;
    let transcript = core.wait_for_run(&conversation, &run).await;

    let calls = transcript.tool_calls(0);
    assert!(
        calls
            .iter()
            .all(|call| call.status == ToolCallStatus::NotRun)
    );
    assert!(calls.iter().all(|call| call.request.is_none()));
    let reasons = format!(
        "{:?}",
        calls.iter().map(|call| &call.not_run).collect::<Vec<_>>()
    );
    assert!(reasons.contains("no trusted host key"), "{reasons}");
    assert!(
        reasons.contains("The registered hosts are: new-box"),
        "{reasons}"
    );
    assert_eq!(server.auth_attempts(), 0);
}

#[tokio::test]
async fn the_model_reads_the_hosts_and_the_tool() {
    let hosted = hosted().await;
    let server = TestServer::start(Options::default()).await.unwrap();
    register(&hosted, "build-box", &server).await;
    let core = &hosted.core;
    core.model.push_reply(reply("I see it."));

    let (conversation, run) = start(core, "OWNER: which hosts?").await;
    core.wait_for_run(&conversation, &run).await;

    let [request] = core.model.requests().try_into().unwrap();
    assert!(
        request.instructions.contains(&format!(
            "- build-box ({USERNAME}@127.0.0.1:{}): The test server.",
            server.port()
        )),
        "{}",
        request.instructions
    );
    assert!(
        request
            .tools
            .iter()
            .any(|tool| tool.name == RUN_SSH_COMMAND)
    );
}
