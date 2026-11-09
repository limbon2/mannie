//! The terminal plugin through the real plugin host, with scripted shells
//! that speak the markers (plugins/terminal/docs/README.md; ADR-0053): the
//! owner's terminals and their commands, an agent's command under the policy
//! with its cited output, the checks before an agent types, the timeout,
//! forged markers, mentions, and the plugin's rules.
//!
//! The owner's terminals run on the core's terminals replaced by a fake; the
//! agents' commands run on a scripted host that a fixture offers through the
//! point `terminal-hosts`, so they pass the core's real registry and policy,
//! and the plugin types the setup as it does on a remote host.

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt as _;
use mannie_plugin_harness::conformance::conformance;
use mannie_plugin_harness::host::{HostOptions, HostedPlugin};
use mannie_plugin_harness::runtime::{
    Harness, HarnessOptions, SeenPart, Transcript, add_plugin, reply, tool_calls,
};
use mannie_plugin_harness::types::{
    Access, ApprovalChoice, ApprovalReason, ApprovalRequest, DecideApproval, MentionRef,
    PartContent, PartId, RunId, SendMessage, StartConversation, TerminalHost, ToolCallStatus,
    ToolCallView,
};
use mannie_plugin_sdk::context::HostProfile;
use mannie_plugin_sdk::contracts::ConversationId;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::host::PluginBinding;
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::ipc::{ChannelMessage, PluginStream};
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::points::terminal_hosts::{PluginTerminalHostSource, TerminalHosts};
use mannie_plugin_sdk::service::Service;
use mannie_plugin_sdk::services::terminals::{ShellKind, Terminals};
use mannie_plugin_sdk::testing::{FakeTerminals, ScriptedHosts};
use mannie_terminal::{RUN_IN_TERMINAL, mannie_plugin};
use serde_json::{Value, json};

/// How long a test waits for something that should come at once.
const WAIT: Duration = Duration::from_secs(10);

/// The plugin that offers the scripted host.
const FIXTURE: &str = "fixture-hosts";

/// The scripted host's name.
const HOST: &str = "build-box";

/// The scripted host's endpoint.
const ENDPOINT: &str = "owner@build-box:22";

fn options(terminals: &FakeTerminals) -> HostOptions {
    let core = HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    };
    HostOptions::new()
        .with_core(core)
        .with_provider(Arc::new(terminals.clone()))
}

/// The plugin, on, with scripted terminals in place of the core's.
async fn hosted(terminals: &FakeTerminals) -> HostedPlugin {
    let hosted = HostedPlugin::new(mannie_plugin(), options(terminals)).await;
    hosted.turn_on().await;
    hosted
}

/// The plugin, on, on the core's terminals, with the scripted host offered.
async fn hosted_with_host() -> (HostedPlugin, ScriptedHosts) {
    let core = HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    };
    let hosted = HostedPlugin::new(mannie_plugin(), HostOptions::new().with_core(core)).await;
    let hosts = ScriptedHosts::new(&[("box-1", HOST, ENDPOINT)]);
    let source = PluginTerminalHostSource::new(FIXTURE, Arc::new(hosts.clone()));
    add_plugin(
        &hosted.core.runtime,
        FIXTURE,
        Contributions::new().add::<TerminalHosts>(source),
    )
    .expect("the core takes the scripted host");
    hosted.turn_on().await;
    (hosted, hosts)
}

async fn start(core: &Harness, text: &str) -> (ConversationId, RunId) {
    let started = core
        .runtime
        .start_conversation(StartConversation::new(text))
        .await
        .expect("the conversation starts");
    (started.conversation_id, started.run_id)
}

/// Opens a terminal of `conversation` where `host` says, as the owner does.
async fn open_where(hosted: &HostedPlugin, conversation: &str, host: Value) -> Value {
    hosted
        .invoke(
            "open_terminal",
            json!({
                "conversationId": conversation,
                "host": host,
                "folder": null,
                "columns": 80,
                "rows": 24,
            }),
        )
        .await
        .expect("the terminal opens")
}

/// Opens a terminal of `conversation` on this computer.
async fn open(hosted: &HostedPlugin, conversation: &str) -> Value {
    open_where(
        hosted,
        conversation,
        json!({ "kind": "this_computer", "shell": null }),
    )
    .await
}

/// Opens a terminal of `conversation` on the scripted host.
async fn open_on_host(hosted: &HostedPlugin, conversation: &str) -> Value {
    open_where(
        hosted,
        conversation,
        json!({ "kind": "plugin", "plugin": FIXTURE, "key": "box-1" }),
    )
    .await
}

async fn type_text(hosted: &HostedPlugin, conversation: &str, terminal: u64, data: &str) {
    hosted
        .invoke(
            "write_terminal",
            json!({ "conversationId": conversation, "terminal": terminal, "data": data }),
        )
        .await
        .expect("the terminal takes the input");
}

/// A view of a terminal.
async fn follow(
    hosted: &HostedPlugin,
    conversation: &str,
    terminal: u64,
) -> PluginStream<ChannelMessage<Value>> {
    hosted
        .open_channel(
            "follow_terminal",
            json!({
                "conversationId": conversation,
                "terminal": terminal,
                "view": "test-view",
                "scrollback": 2000,
            }),
        )
        .await
        .expect("the view opens")
}

/// Reads a view's updates until one matches, and returns it.
async fn until(
    view: &mut PluginStream<ChannelMessage<Value>>,
    matches: impl Fn(&Value) -> bool,
) -> Value {
    tokio::time::timeout(WAIT, async {
        loop {
            match view.next().await {
                Some(ChannelMessage::Item { item }) if matches(&item) => return Some(item),
                Some(ChannelMessage::Item { .. }) => {}
                _ended => return None,
            }
        }
    })
    .await
    .expect("the update comes in time")
    .expect("the view stays open")
}

fn is_prompt(update: &Value) -> bool {
    update["kind"] == "status" && update["phase"] == "prompt" && update["marking"] == true
}

fn ended_block(update: &Value, command: &str) -> bool {
    update["kind"] == "block"
        && update["block"]["command"] == command
        && update["block"]["running"] == false
}

/// The tool call that waits for the owner, if one does.
fn waiting_call(transcript: &Transcript, turn: u32) -> Option<(PartId, ApprovalRequest)> {
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

/// Waits until a call of `turn` waits for the owner, and answers it with
/// `choice`.
async fn answer(
    core: &Harness,
    conversation: &ConversationId,
    turn: u32,
    choice: ApprovalChoice,
) -> ApprovalRequest {
    let transcript = core
        .wait_until(conversation, |transcript| {
            waiting_call(transcript, turn).is_some()
        })
        .await;
    let (part_id, request) = waiting_call(&transcript, turn).expect("a call waits for the owner");
    core.runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation.clone(),
            part_id: part_id.clone(),
            digest: request.digest.clone(),
            choice,
        })
        .await
        .expect("the decision is taken");
    core.wait_until(conversation, |transcript| {
        waiting_call(transcript, turn).is_none_or(|(waiting, _request)| waiting != part_id)
    })
    .await;
    request
}

fn run_arguments(command: &str, terminal: Option<u64>, timeout_seconds: Option<u32>) -> String {
    let host = terminal.is_none().then_some(HOST);
    json!({
        "activity": "Running",
        "command": command,
        "terminal": terminal,
        "host": host,
        "shell": null,
        "timeout_seconds": timeout_seconds,
    })
    .to_string()
}

fn output(call: &ToolCallView) -> &str {
    &call.result.as_ref().expect("the call ran").output
}

#[tokio::test]
async fn the_plugin_keeps_the_rules_of_the_sdk() {
    let terminals = FakeTerminals::new();

    assert_eq!(
        conformance(mannie_plugin(), options(&terminals)).await,
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn the_owner_opens_a_terminal_and_its_commands_become_blocks() {
    let terminals = FakeTerminals::new();
    let hosted = hosted(&terminals).await;
    let (conversation, _run) = start(&hosted.core, "OWNER: hello").await;
    let conversation = conversation.to_string();

    let opened = open(&hosted, &conversation).await;
    let mut view = follow(&hosted, &conversation, 1).await;
    let snapshot = until(&mut view, |update| update["kind"] == "snapshot").await;
    until(&mut view, is_prompt).await;
    type_text(&hosted, &conversation, 1, "echo hello\r").await;
    let block = until(&mut view, |update| ended_block(update, "echo hello")).await;
    let text = hosted
        .invoke(
            "block_text",
            json!({ "conversationId": conversation, "terminal": 1, "block": 1 }),
        )
        .await
        .expect("the text of the command is read");

    assert_eq!(opened["number"], 1);
    assert_eq!(opened["openedBy"], "owner");
    assert!(snapshot["seq"].as_u64().is_some());
    assert_eq!(block["block"]["author"], "owner");
    assert_eq!(block["block"]["exitCode"], 0);
    assert_eq!(text["lines"], json!(["hello"]));
    let started = terminals.opened();
    assert_eq!(started.len(), 1);
    assert!(
        started[0]
            .setup
            .as_deref()
            .is_some_and(|setup| setup.contains("633;C"))
    );
}

#[tokio::test]
async fn a_forged_marker_ends_no_command() {
    let terminals = FakeTerminals::new();
    let hosted = hosted(&terminals).await;
    let (conversation, _run) = start(&hosted.core, "OWNER: hello").await;
    let conversation = conversation.to_string();
    open(&hosted, &conversation).await;
    let mut view = follow(&hosted, &conversation, 1).await;
    until(&mut view, is_prompt).await;

    type_text(&hosted, &conversation, 1, "forge\r").await;
    until(&mut view, |update| ended_block(update, "forge")).await;
    let text = hosted
        .invoke(
            "block_text",
            json!({ "conversationId": conversation, "terminal": 1, "block": 1 }),
        )
        .await
        .expect("the text of the command is read");

    assert_eq!(text["lines"], json!(["forged"]));
}

#[tokio::test]
async fn an_agent_runs_a_command_once_the_owner_allows_it_and_cites_its_output() {
    let (hosted, hosts) = hosted_with_host().await;
    let core = &hosted.core;
    core.model.push_reply(tool_calls(&[(
        "call_1",
        RUN_IN_TERMINAL,
        &run_arguments("lines 3", None, None),
    )]));
    core.model.push_reply(reply("Three lines."));

    let (conversation, run) = start(core, "OWNER: print three lines").await;
    let request = answer(core, &conversation, 0, ApprovalChoice::AllowOnce).await;
    let transcript = core.wait_for_run(&conversation, &run).await;

    assert_eq!(request.reasons, vec![ApprovalReason::TerminalCommand]);
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert!(
        matches!(
            &call.accesses[..],
            [Access::TerminalCommand {
                host: TerminalHost::Remote { plugin, label, endpoint, .. },
                terminal: None,
                command,
                ..
            }] if plugin == FIXTURE && label == HOST && endpoint == ENDPOINT && command == "lines 3"
        ),
        "{:?}",
        call.accesses
    );
    assert_eq!(hosts.opened(), ["box-1"]);
    let text = output(&call);
    assert!(
        text.starts_with(
            "Ran `lines 3` in terminal 1 (bash on build-box), in /home/owner. It exited with 0"
        ),
        "{text}"
    );
    assert!(text.contains("(cite:terminal/"), "{text}");
    assert!(
        text.contains("     1\tline 1\n     2\tline 2\n     3\tline 3"),
        "{text}"
    );
    let [cited] = call.cited.as_slice() else {
        panic!("one cited item: {:?}", call.cited);
    };
    assert_eq!(cited.provider, "terminal");
    assert_eq!(cited.title, "lines 3");
    assert_eq!(cited.text, "line 1\nline 2\nline 3");
    // The terminal that the agent opened belongs to the conversation.
    let list = hosted
        .open_channel(
            "follow_terminals",
            json!({ "conversationId": conversation.to_string() }),
        )
        .await
        .expect("the list opens");
    let first = list.take(1).collect::<Vec<_>>().await;
    let ChannelMessage::Item { item } = &first[0] else {
        panic!("{first:?}");
    };
    assert_eq!(item["terminals"][0]["openedBy"], "agent");
}

#[tokio::test]
async fn an_agent_never_types_while_the_owner_types() {
    let (hosted, _hosts) = hosted_with_host().await;
    let core = &hosted.core;
    let (conversation, run) = start(core, "OWNER: hello").await;
    core.wait_for_run(&conversation, &run).await;
    let id = conversation.to_string();
    open_on_host(&hosted, &id).await;
    let mut view = follow(&hosted, &id, 1).await;
    until(&mut view, is_prompt).await;
    type_text(&hosted, &id, 1, "ech").await;
    core.model.push_reply(tool_calls(&[(
        "call_1",
        RUN_IN_TERMINAL,
        &run_arguments("echo hi", Some(1), None),
    )]));
    core.model.push_reply(reply("It waits."));

    let started = core
        .runtime
        .send_message(SendMessage::new(
            conversation.clone(),
            "OWNER: run it there",
        ))
        .await
        .expect("the message is sent");
    answer(core, &conversation, 1, ApprovalChoice::AllowOnce).await;
    let transcript = core.wait_for_run(&conversation, &started.run_id).await;

    let calls = transcript.tool_calls(1);
    let [call] = calls.as_slice() else {
        panic!("{calls:?}");
    };
    assert!(
        output(call).contains("The owner is typing in terminal 1"),
        "{}",
        output(call)
    );
}

#[tokio::test]
async fn the_timeout_interrupts_the_command_with_ctrl_c() {
    let (hosted, _hosts) = hosted_with_host().await;
    let core = &hosted.core;
    core.model.push_reply(tool_calls(&[(
        "call_1",
        RUN_IN_TERMINAL,
        &run_arguments("sleep", None, Some(1)),
    )]));
    core.model.push_reply(reply("It was interrupted."));

    let (conversation, run) = start(core, "OWNER: sleep").await;
    answer(core, &conversation, 0, ApprovalChoice::AllowOnce).await;
    let transcript = core.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert!(
        output(&call).contains("interrupted with Ctrl+C"),
        "{}",
        output(&call)
    );
    assert_eq!(core.runtime.running_terminal_count(), 0);
}

#[tokio::test]
async fn a_mention_of_a_command_gives_the_model_its_output() {
    let terminals = FakeTerminals::new();
    let hosted = hosted(&terminals).await;
    let core = &hosted.core;
    let (conversation, run) = start(core, "OWNER: hello").await;
    core.wait_for_run(&conversation, &run).await;
    let id = conversation.to_string();
    open(&hosted, &id).await;
    let mut view = follow(&hosted, &id, 1).await;
    until(&mut view, is_prompt).await;
    type_text(&hosted, &id, 1, "fail\r").await;
    until(&mut view, |update| ended_block(update, "fail")).await;

    let entry = hosted
        .invoke(
            "mention_block",
            json!({ "conversationId": id, "terminal": 1, "block": 1 }),
        )
        .await
        .expect("the mention of the command");
    core.model.push_reply(reply("It failed."));
    let message = SendMessage::new(conversation.clone(), "OWNER: why did this fail?")
        .with_mentions(vec![MentionRef {
            provider: "terminal".to_owned(),
            key: entry["key"].as_str().expect("a key").to_owned(),
        }]);
    let started = core
        .runtime
        .send_message(message)
        .await
        .expect("the message is sent");
    core.wait_for_run(&conversation, &started.run_id).await;

    let requests = core.model.requests();
    let sent = format!("{:?}", requests.last().expect("a request").messages);
    assert!(sent.contains("which exited with 1"), "{sent}");
    assert!(sent.contains("error: it failed"), "{sent}");
    assert_eq!(entry["token"], "@terminal-1:1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_supported_shell_of_this_computer_marks_its_commands() {
    let core = HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    };
    let hosted = HostedPlugin::new(mannie_plugin(), HostOptions::new().with_core(core)).await;
    hosted.turn_on().await;
    let (conversation, _run) = start(&hosted.core, "OWNER: hello").await;
    let id = conversation.to_string();
    let provider = hosted
        .core
        .runtime
        .service_providers()
        .into_iter()
        .find(|provider| provider.name() == Terminals::NAME)
        .expect("the core provides terminals");
    let plugin = PluginId::parse(hosted.id()).unwrap();
    let manifest = Manifest::parse(mannie_plugin().manifest).unwrap();
    let terminals = provider
        .bind(&PluginBinding::new(
            &plugin,
            &manifest,
            HostProfile::Development,
        ))
        .expect("the core binds terminals to the plugin")
        .get::<Terminals>()
        .expect("the binding is the service terminals");
    let list = terminals
        .hosts()
        .next()
        .await
        .expect("the list of hosts comes");
    // /etc/shells can include programs such as screen, which open without markers.
    let shells = list
        .shells
        .iter()
        .filter(|shell| shell.kind != ShellKind::Other);
    assert!(
        shells.clone().next().is_some(),
        "this computer lists a supported shell"
    );

    for (number, shell) in (1_u64..).zip(shells) {
        let program = shell.program.to_string_lossy();
        let name = &shell.name;
        open_where(
            &hosted,
            &id,
            json!({ "kind": "this_computer", "shell": program }),
        )
        .await;
        let mut view = follow(&hosted, &id, number).await;
        let marked = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                match view.next().await {
                    Some(ChannelMessage::Item { item }) if is_prompt(&item) => return true,
                    Some(ChannelMessage::Item { .. }) => {}
                    _ended => return false,
                }
            }
        })
        .await
        .unwrap_or(false);
        assert!(marked, "{name} reaches a prompt that marks commands");
        type_text(&hosted, &id, number, "echo hello\r").await;
        let block = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                match view.next().await {
                    Some(ChannelMessage::Item { item })
                        if item["kind"] == "block" && item["block"]["running"] == false =>
                    {
                        return Some(item);
                    }
                    Some(ChannelMessage::Item { .. }) => {}
                    _ended => return None,
                }
            }
        })
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| panic!("{name} ends the command"));
        let text = hosted
            .invoke(
                "block_text",
                json!({ "conversationId": id, "terminal": number, "block": block["block"]["number"] }),
            )
            .await
            .expect("the text of the command is read");
        assert_eq!(text["lines"], json!(["hello"]), "{name}");
        hosted
            .invoke(
                "end_terminal",
                json!({ "conversationId": id, "terminal": number }),
            )
            .await
            .expect("the terminal ends");
    }
}

#[tokio::test]
async fn turning_the_plugin_off_ends_its_terminals() {
    let terminals = FakeTerminals::new();
    let hosted = hosted(&terminals).await;
    let (conversation, _run) = start(&hosted.core, "OWNER: hello").await;
    let id = conversation.to_string();
    open(&hosted, &id).await;
    let mut view = follow(&hosted, &id, 1).await;
    until(&mut view, is_prompt).await;

    hosted.turn_off().await;
    let mut ended = false;
    let _finished = tokio::time::timeout(WAIT, async {
        while let Some(message) = view.next().await {
            match message {
                ChannelMessage::Item { item } if item["kind"] == "ended" => ended = true,
                ChannelMessage::Item { .. } => {}
                ChannelMessage::Ended { .. } => break,
            }
        }
    })
    .await;

    // The view hears the end, or the host ends the channel with the plugin.
    assert!(ended || terminals.opened().len() == 1);
    assert_eq!(hosted.core.runtime.running_terminal_count(), 0);
}
