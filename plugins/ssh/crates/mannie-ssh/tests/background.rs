//! The tool `start_ssh_command` through the core's tool loop, with a
//! scripted model and an SSH server in the test's process (plugins/ssh/
//! docs/README.md, the tool; ADR-0041, decision 11): a command keeps running
//! after its call, `read_command_output` reads its lines as they arrive and
//! `stop_command` ends it with TERM, the password never shows in its lines,
//! a command that ends on the host says how, and editing the host or turning
//! SSH off ends it.

use std::sync::Arc;
use std::time::Duration;

use mannie_plugin_harness::host::{HostOptions, HostedPlugin};
use mannie_plugin_harness::provider::{Content, ModelRequest, Role};
use mannie_plugin_harness::runtime::{Harness, HarnessOptions, Transcript, reply, tool_calls};
use mannie_plugin_harness::types::{
    BackgroundEnding, BackgroundSource, RunId, StartConversation, StopBackgroundCommand,
    UpdateToolsSettings,
};
use mannie_plugin_sdk::contracts::{ConversationId, WorkItem};
use mannie_plugin_sdk::services::background_commands::BACKGROUND_COMMANDS_WORK;
use mannie_plugin_sdk::testing::FakeSecrets;
use mannie_ssh::{START_SSH_COMMAND, mannie_plugin};
use mannie_ssh_testing::{Options, PASSWORD, TestServer, USERNAME};
use serde_json::json;

/// The plugin, on, in a runtime whose conversations use tools in full
/// access, so commands run without asking, and whose commands that end on
/// their own wake the agent only when `wake` says so.
async fn hosted(wake: bool) -> HostedPlugin {
    let core = HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    };
    let options = HostOptions::new()
        .with_core(core)
        .with_provider(Arc::new(FakeSecrets::new()));
    let hosted = HostedPlugin::new(mannie_plugin(), options).await;
    hosted
        .core
        .runtime
        .update_tools_settings(UpdateToolsSettings {
            allow_changes_in_workspace: true,
            command_environment: Vec::new(),
            full_access_for_new_conversations: true,
            wake_on_command_end: wake,
        })
        .await
        .expect("the settings are kept");
    hosted.turn_on().await;
    hosted
}

/// The fields of a host of `server` named `name`, with `note`.
fn host_fields(name: &str, server: &TestServer, note: &str) -> serde_json::Value {
    json!({
        "name": name,
        "address": "127.0.0.1",
        "port": server.port(),
        "username": USERNAME,
        "password": PASSWORD,
        "note": note,
    })
}

/// Registers a host of `server` and trusts its key, as the owner does in
/// the settings. Returns the host's ID.
async fn register(hosted: &HostedPlugin, name: &str, server: &TestServer) -> String {
    let added = hosted
        .invoke("add_host", host_fields(name, server, "The test server."))
        .await
        .expect("the host is added");
    let id = added["id"].as_str().expect("a host has an ID").to_owned();
    let scan = hosted
        .invoke("scan_host_key", json!({ "hostId": id }))
        .await
        .expect("the host's key is scanned");
    hosted
        .invoke(
            "trust_host_key",
            json!({ "hostId": id, "fingerprint": scan["key"]["fingerprint"] }),
        )
        .await
        .expect("the scanned key is trusted");
    id
}

/// The arguments of a start on `host`.
fn on(host: &str, command: &str) -> String {
    json!({ "activity": "Following", "host": host, "command": command }).to_string()
}

async fn start(core: &Harness, text: &str) -> (ConversationId, RunId) {
    let started = core
        .runtime
        .start_conversation(StartConversation::new(text))
        .await
        .expect("the conversation starts");
    (started.conversation_id, started.run_id)
}

/// The result of the call `call_id` in a request.
fn result(request: &ModelRequest, call_id: &str) -> String {
    request
        .messages
        .iter()
        .filter(|message| message.role == Role::Tool)
        .flat_map(|message| &message.content)
        .find_map(|content| match content {
            Content::ToolResult {
                call_id: id,
                output,
            } if id == call_id => Some(output.clone()),
            _ => None,
        })
        .expect("the request holds the call's result")
}

/// Waits until command `number` of the conversation ended.
async fn ended(core: &Harness, conversation: &ConversationId, number: u32) -> Transcript {
    core.wait_until(conversation, |transcript| {
        transcript
            .background_commands
            .get(&number)
            .is_some_and(|command| command.ending.is_some())
    })
    .await
}

/// Starts `command` on the host `test-box` in a conversation whose first
/// reply only starts it, and waits until that reply ended.
async fn started(core: &Harness, command: &str) -> ConversationId {
    core.model.push_reply(tool_calls(&[(
        "call_1",
        START_SSH_COMMAND,
        &on("test-box", command),
    )]));
    core.model.push_reply(reply("It runs."));
    let (conversation, run) = start(core, "Follow the test box").await;
    core.wait_for_run(&conversation, &run).await;
    conversation
}

#[tokio::test]
async fn a_command_keeps_running_its_lines_arrive_for_reads_and_stop_command_ends_it_with_term() {
    let hosted = hosted(false).await;
    let server = TestServer::start(Options::default()).await.unwrap();
    register(&hosted, "test-box", &server).await;
    let core = &hosted.core;
    core.model.push_reply(tool_calls(&[(
        "call_1",
        START_SSH_COMMAND,
        &on("test-box", "drip 3"),
    )]));
    core.model.push_reply(tool_calls(&[(
        "call_2",
        "read_command_output",
        &json!({ "command": 1, "wait_seconds": 20, "until": "^line 3$" }).to_string(),
    )]));
    core.model.push_reply(tool_calls(&[(
        "call_3",
        "stop_command",
        &json!({ "command": 1 }).to_string(),
    )]));
    core.model
        .push_reply(reply("It wrote three lines, and it is stopped."));

    let (conversation, run) = start(core, "Follow the test box").await;
    let transcript = core.wait_for_run(&conversation, &run).await;

    let requests = core.model.reply_requests();
    let endpoint = format!("{USERNAME}@127.0.0.1:{}", server.port());
    let started = result(&requests[1], "call_1");
    assert!(
        started.starts_with(&format!(
            "Command 1 started in the background on test-box ({endpoint})."
        )),
        "{started}"
    );
    let read = result(&requests[2], "call_2");
    assert!(read.contains("line 1\nline 2\nline 3\n"), "{read}");
    assert!(read.contains("Line 3 matches until."), "{read}");
    let stopped = result(&requests[3], "call_3");
    assert!(stopped.contains("is stopped."), "{stopped}");
    assert_eq!(server.signals(), ["TERM"]);
    let command = &transcript.background_commands[&1];
    assert_eq!(command.ending, Some(BackgroundEnding::StoppedByAgent));
    assert_eq!(command.command, "drip 3");
    assert_eq!(
        command.source,
        BackgroundSource::Remote {
            plugin: "ssh".to_owned(),
            label: "test-box".to_owned(),
            endpoint,
        }
    );
    core.runtime.shutdown().await;
}

#[tokio::test]
async fn the_password_never_shows_in_the_lines_even_when_two_writes_cut_it() {
    let hosted = hosted(false).await;
    let server = TestServer::start(Options::default()).await.unwrap();
    register(&hosted, "test-box", &server).await;
    let core = &hosted.core;
    let conversation = started(core, "drip-password").await;
    core.model.push_reply(tool_calls(&[(
        "call_2",
        "read_command_output",
        &json!({ "command": 1, "wait_seconds": 20, "until": "password" }).to_string(),
    )]));
    core.model
        .push_reply(reply("It printed a redacted password."));

    let run = core
        .runtime
        .send_message(mannie_plugin_harness::types::SendMessage::new(
            conversation.clone(),
            "What did it print?",
        ))
        .await
        .unwrap()
        .run_id;
    core.wait_for_run(&conversation, &run).await;
    let stopped = core
        .runtime
        .stop_background_command(StopBackgroundCommand {
            conversation_id: conversation.clone(),
            number: 1,
        })
        .await
        .unwrap();

    let read = result(&core.model.reply_requests()[3], "call_2");
    assert!(read.contains("the password is [REDACTED]\n"), "{read}");
    assert!(!read.contains(PASSWORD), "{read}");
    assert!(!read.contains(&PASSWORD[..7]), "{read}");
    assert_eq!(stopped.ending, Some(BackgroundEnding::StoppedByOwner));
    let stored = core
        .store
        .background_command(&conversation, 1)
        .await
        .unwrap()
        .unwrap();
    let archive = core
        .store
        .archive(&conversation, stored.document.archive_id.as_ref().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(archive.content, "the password is [REDACTED]\n");
    core.runtime.shutdown().await;
}

#[tokio::test]
async fn a_command_that_ends_on_the_host_says_how() {
    let hosted = hosted(false).await;
    let server = TestServer::start(Options::default()).await.unwrap();
    register(&hosted, "test-box", &server).await;
    let core = &hosted.core;

    let conversation = started(core, "count 2").await;
    let transcript = ended(core, &conversation, 1).await;

    let command = &transcript.background_commands[&1];
    assert_eq!(command.ending, Some(BackgroundEnding::EndedByHost));
    assert_eq!(command.exit_code, Some(0));
    assert_eq!(command.lines, 2);
    assert_eq!(server.signals(), Vec::<String>::new());
    core.runtime.shutdown().await;
}

#[tokio::test]
async fn editing_the_host_ends_its_command_as_its_session_closed_without_a_wake_up() {
    let hosted = hosted(true).await;
    let server = TestServer::start(Options::default()).await.unwrap();
    let id = register(&hosted, "test-box", &server).await;
    let core = &hosted.core;
    let conversation = started(core, "drip 1").await;

    let mut fields = host_fields("test-box", &server, "Edited.");
    fields["hostId"] = json!(id);
    fields["password"] = json!(null);
    hosted
        .invoke("update_host", fields)
        .await
        .expect("the host is edited");
    let transcript = ended(core, &conversation, 1).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert_eq!(
        transcript.background_commands[&1].ending,
        Some(BackgroundEnding::SessionClosed)
    );
    assert_eq!(core.snapshot(&conversation).await.turns.len(), 1);
    core.runtime.shutdown().await;
}

#[tokio::test]
async fn turning_ssh_off_ends_its_commands_after_the_switch_names_them() {
    let hosted = hosted(false).await;
    let server = TestServer::start(Options::default()).await.unwrap();
    register(&hosted, "test-box", &server).await;
    let core = &hosted.core;
    let conversation = started(core, "drip 1").await;

    let work = hosted.host.work("ssh").await.unwrap();
    hosted.turn_off().await;
    let transcript = ended(core, &conversation, 1).await;

    assert_eq!(work.items, vec![WorkItem::new(BACKGROUND_COMMANDS_WORK, 1)]);
    assert_eq!(
        transcript.background_commands[&1].ending,
        Some(BackgroundEnding::PluginOff)
    );
    assert_eq!(server.signals(), ["TERM"]);
    core.runtime.shutdown().await;
}

#[tokio::test]
async fn a_command_that_ignores_term_gets_kill_and_then_loses_its_channel() {
    let hosted = hosted(false).await;
    let server = TestServer::start(Options::default()).await.unwrap();
    register(&hosted, "test-box", &server).await;
    let core = &hosted.core;
    let conversation = started(core, "ignore-signals").await;

    let stopped = core
        .runtime
        .stop_background_command(StopBackgroundCommand {
            conversation_id: conversation.clone(),
            number: 1,
        })
        .await
        .unwrap();

    assert_eq!(stopped.ending, Some(BackgroundEnding::StoppedByOwner));
    assert_eq!(server.signals(), ["TERM", "KILL"]);
    core.runtime.shutdown().await;
}
