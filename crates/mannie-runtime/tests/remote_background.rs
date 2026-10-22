//! Commands that a plugin's tool starts on a remote host in the background
//! and hands the core through the service `background-commands` (ADR-0041,
//! decision 11): a call with such an access gets a ticket for one start
//! while it executes, the core follows the command as it follows a local
//! one, and its plugin ends it when the agent or the owner stops it, when
//! the plugin goes off and when Mannie quits.

#![cfg(test)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::stream::BoxStream;
use futures_util::{FutureExt, StreamExt};
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::WorkItem;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::points::tools::{
    AgentSelector, PluginTool, RemoteCommandAccess, Tool, ToolAccess, ToolCall, ToolExecution,
    ToolRefusal, ToolResult, ToolSpec, Tools,
};
use mannie_plugin_sdk::services::background_commands::{
    BACKGROUND_COMMANDS_WORK, BackgroundCommandsApi, BackgroundError, BackgroundScope,
    BackgroundTicket, RemoteCommand, RemoteEnding, RemoteOutput,
};
use mannie_provider::{Content, ModelRequest, Role};
use mannie_runtime::RuntimeConfig;
use mannie_runtime::testing::{
    Harness, HarnessOptions, Transcript, add_plugin, background_commands, fast_config, reply,
    tool_calls, turn_off_plugin,
};
use mannie_types::chat::{SendMessage, StartConversation};
use mannie_types::ids::{ConversationId, RunId};
use mannie_types::tools::{
    BackgroundEnding, BackgroundSource, StopBackgroundCommand, ToolCallStatus, ToolOutcome,
    UpdateToolsSettings,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// The fake plugin's ID.
const PLUGIN: &str = "shell";

/// The fake plugin's tool.
const START_ON_HOST: &str = "start_on_host";

/// The endpoint of every host of the fake plugin.
const ENDPOINT: &str = "deploy@build-box.lan:22";

/// The pinned key of every host of the fake plugin.
const IDENTITY: &str = "SHA256:vh3fjoyg7pBnxLhORIqZjyJQqYYN/gzBTuj+mPdeJlo";

/// The most items that a fake command's output holds before the core reads
/// them, more than any test writes.
const OUTPUT_ROOM: usize = 64;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap()
}

/// The host's side of a fake remote command: the test writes its lines and
/// its end, and a stop of the core closes it.
#[derive(Debug, Clone, Default)]
struct Host {
    sender: Arc<Mutex<Option<mpsc::Sender<RemoteOutput>>>>,
    stops: Arc<AtomicUsize>,
}

impl Host {
    fn write(&self, lines: &[&str]) {
        let lines = lines.iter().map(|line| (*line).to_owned()).collect();
        let sender = lock(&self.sender).clone();
        if let Some(sender) = sender {
            sender.try_send(RemoteOutput::lines(lines)).unwrap();
        }
    }

    /// Ends the command on the host, which says how.
    fn end(&self, ending: RemoteEnding) {
        let sender = lock(&self.sender).take();
        if let Some(sender) = sender {
            sender.try_send(RemoteOutput::ended(ending)).unwrap();
        }
    }

    /// Closes the output without saying how the command ended.
    fn let_go(&self) {
        lock(&self.sender).take();
    }

    fn stops(&self) -> usize {
        self.stops.load(Ordering::SeqCst)
    }
}

/// A remote command whose output the test writes through its [`Host`].
#[derive(Debug)]
struct FakeCommand {
    output: Option<mpsc::Receiver<RemoteOutput>>,
    host: Host,
}

impl FakeCommand {
    fn new() -> (Self, Host) {
        let (sender, receiver) = mpsc::channel(OUTPUT_ROOM);
        let host = Host::default();
        *lock(&host.sender) = Some(sender);
        let command = Self {
            output: Some(receiver),
            host: host.clone(),
        };
        (command, host)
    }
}

impl RemoteCommand for FakeCommand {
    fn output(&mut self) -> BoxStream<'static, RemoteOutput> {
        let receiver = self.output.take().expect("the core takes the output once");
        futures_util::stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|item| (item, receiver))
        })
        .boxed()
    }

    fn stop(self: Box<Self>) -> BoxFuture<'static, RemoteEnding> {
        self.host.stops.fetch_add(1, Ordering::SeqCst);
        self.host.let_go();
        async {
            RemoteEnding::Signaled {
                signal: "TERM".to_owned(),
            }
        }
        .boxed()
    }
}

/// What a call of the fake tool does with its ticket.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Behavior {
    /// It starts one command.
    #[default]
    StartsOnce,
    /// It tries the ticket with another plugin's service, starts with its
    /// own, and tries the ticket again.
    TriesTheTicket,
    /// Its preparation returns the command in the foreground.
    Foreground,
}

/// What the fake tool saw and did.
#[derive(Debug, Default)]
struct Seen {
    tickets: Vec<Option<BackgroundTicket>>,
    hosts: Vec<Host>,
    refusals: Vec<BackgroundError>,
}

/// A plugin's tool that starts a command on a host in the background, as
/// the plugin SSH's `start_ssh_command` does.
#[derive(Debug)]
struct StartOnHost {
    commands: Arc<dyn BackgroundCommandsApi>,
    other: Arc<dyn BackgroundCommandsApi>,
    behavior: Mutex<Behavior>,
    seen: Arc<Mutex<Seen>>,
}

impl StartOnHost {
    fn new(harness: &Harness) -> Arc<Self> {
        Arc::new(Self {
            commands: background_commands(&harness.runtime, PLUGIN),
            other: background_commands(&harness.runtime, "other"),
            behavior: Mutex::default(),
            seen: Arc::default(),
        })
    }

    fn behaving(&self, behavior: Behavior) {
        *lock(&self.behavior) = behavior;
    }

    fn host(&self, index: usize) -> Host {
        lock(&self.seen).hosts[index].clone()
    }
}

impl Tool for StartOnHost {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            START_ON_HOST,
            "Starts a command on a host in the background.",
            json!({
                "additionalProperties": false,
                "properties": {
                    "host": { "type": "string" },
                    "command": { "type": "string" },
                },
                "required": ["host", "command"],
                "type": "object",
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        let foreground = *lock(&self.behavior) == Behavior::Foreground;
        async move {
            let arguments: Value = serde_json::from_str(&call.arguments)
                .map_err(|error| ToolRefusal::new(error.to_string()))?;
            let host = arguments["host"].as_str().unwrap_or_default().to_owned();
            let command = arguments["command"].as_str().unwrap_or_default().to_owned();
            let access = RemoteCommandAccess::new(
                format!("key-of-{host}"),
                host,
                ENDPOINT.to_owned(),
                IDENTITY.to_owned(),
                command,
            );
            let access = if foreground {
                access
            } else {
                access.in_background()
            };
            Ok(vec![ToolAccess::RemoteCommand(access)])
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let behavior = *lock(&self.behavior);
        let commands = Arc::clone(&self.commands);
        let other = Arc::clone(&self.other);
        let seen = Arc::clone(&self.seen);
        lock(&seen).tickets.push(execution.background.clone());
        async move {
            let Some(ticket) = execution.background else {
                return ToolResult::new("No ticket.").with_failure("no_ticket");
            };
            if behavior == Behavior::TriesTheTicket {
                let (command, host) = FakeCommand::new();
                lock(&seen).hosts.push(host);
                let refused = other.start(ticket.clone(), Box::new(command)).await;
                lock(&seen).refusals.push(refused.unwrap_err());
            }
            let (command, host) = FakeCommand::new();
            lock(&seen).hosts.push(host);
            let started = commands.start(ticket.clone(), Box::new(command)).await;
            if behavior == Behavior::TriesTheTicket {
                let (command, host) = FakeCommand::new();
                lock(&seen).hosts.push(host);
                let refused = commands.start(ticket, Box::new(command)).await;
                lock(&seen).refusals.push(refused.unwrap_err());
            }
            match started {
                Ok(started) => ToolResult::new(format!("Command {} started.", started.number)),
                Err(error) => {
                    lock(&seen).refusals.push(error.clone());
                    ToolResult::new(format!("Not started: {error}.")).with_failure("refused")
                }
            }
        }
        .boxed()
    }
}

/// The fake plugin with its tool, which has side effects and is offered to
/// the owner's conversations.
fn with_tool(tool: &Arc<StartOnHost>) -> Contributions {
    let tool: Arc<dyn Tool> = Arc::<StartOnHost>::clone(tool);
    Contributions::new()
        .add::<Tools>(PluginTool::new(tool, AgentSelector::Conversations).with_side_effects())
}

/// The arguments of a call of the fake tool.
fn on(host: &str, command: &str) -> String {
    json!({ "activity": "Starting", "host": host, "command": command }).to_string()
}

/// A runtime whose profile uses tools and whose conversations start in full
/// access, so calls run without asking, with the fake plugin's tool.
async fn setup(wake: bool) -> (Harness, Arc<StartOnHost>) {
    let harness = Harness::with_options(HarnessOptions {
        use_tools: true,
        config: RuntimeConfig {
            wake_up_delay: Duration::from_millis(50),
            ..fast_config()
        },
        ..HarnessOptions::default()
    })
    .await;
    harness
        .runtime
        .update_tools_settings(UpdateToolsSettings {
            allow_changes_in_workspace: true,
            command_environment: Vec::new(),
            full_access_for_new_conversations: true,
            wake_on_command_end: wake,
        })
        .await
        .unwrap();
    let tool = StartOnHost::new(&harness);
    add_plugin(&harness.runtime, PLUGIN, with_tool(&tool)).unwrap();
    (harness, tool)
}

async fn start(harness: &Harness, text: &str) -> (ConversationId, RunId) {
    let started = harness
        .runtime
        .start_conversation(StartConversation::new(text))
        .await
        .unwrap();
    (started.conversation_id, started.run_id)
}

/// Waits until command `number` of the conversation ended.
async fn ended(harness: &Harness, conversation: &ConversationId, number: u32) -> Transcript {
    harness
        .wait_until(conversation, |transcript| {
            transcript
                .background_commands
                .get(&number)
                .is_some_and(|command| command.ending.is_some())
        })
        .await
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
        .unwrap_or_else(|| panic!("no result for {call_id}"))
}

/// The work in progress that the runtime keeps for the fake plugin.
fn work(harness: &Harness) -> Vec<WorkItem> {
    let plugin = PluginId::parse(PLUGIN).unwrap();
    harness
        .runtime
        .contribution_consumers()
        .iter()
        .flat_map(|consumer| consumer.work(&plugin))
        .collect()
}

#[tokio::test]
async fn a_plugin_s_command_runs_past_its_call_and_reads_and_stops_like_a_local_one() {
    let (harness, tool) = setup(false).await;
    let model = &harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        START_ON_HOST,
        &on("build-box", "journalctl -f"),
    )]));
    model.push_reply(reply("It follows the journal."));
    model.push_reply(tool_calls(&[(
        "call_2",
        "read_command_output",
        &json!({ "command": 1, "wait_seconds": 30, "until": "^ready" }).to_string(),
    )]));
    model.push_reply(reply("It is ready."));

    let (conversation, first) = start(&harness, "Follow the journal on build-box").await;
    let transcript = harness.wait_for_run(&conversation, &first).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(
        call.result.unwrap().outcome,
        ToolOutcome::Plugin {
            plugin: PLUGIN.to_owned(),
            data: None,
            failure: None,
        }
    );
    let command = &transcript.background_commands[&1];
    assert!(command.is_running());
    assert_eq!(command.command, "journalctl -f");
    assert_eq!(
        command.source,
        BackgroundSource::Remote {
            plugin: PLUGIN.to_owned(),
            label: "build-box".to_owned(),
            endpoint: ENDPOINT.to_owned(),
        }
    );
    assert_eq!(harness.runtime.running_background_count(), 1);
    assert_eq!(
        work(&harness),
        vec![WorkItem::new(BACKGROUND_COMMANDS_WORK, 1)]
    );
    let host = tool.host(0);
    host.write(&["booting", "ready on 8080"]);

    let second = harness
        .runtime
        .send_message(SendMessage::new(conversation.clone(), "Is it ready?"))
        .await
        .unwrap()
        .run_id;
    harness.wait_for_run(&conversation, &second).await;
    let stopped = harness
        .runtime
        .stop_background_command(StopBackgroundCommand {
            conversation_id: conversation.clone(),
            number: 1,
        })
        .await
        .unwrap();

    let requests = model.reply_requests();
    let read = result(&requests[3], "call_2");
    assert!(read.contains("booting\nready on 8080\n"), "{read}");
    assert!(read.contains("Line 2 matches until."), "{read}");
    assert_eq!(stopped.ending, Some(BackgroundEnding::StoppedByOwner));
    assert_eq!(stopped.signal, None);
    assert_eq!(host.stops(), 1);
    assert_eq!(harness.runtime.running_background_count(), 0);
    assert_eq!(
        work(&harness),
        vec![WorkItem::new(BACKGROUND_COMMANDS_WORK, 0)]
    );
    let stored = harness
        .store
        .background_command(&conversation, 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.document.process, None);
    let archive = harness
        .store
        .archive(&conversation, stored.document.archive_id.as_ref().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(archive.content, "booting\nready on 8080\n");
    harness.runtime.shutdown().await;
}

#[tokio::test]
async fn a_ticket_starts_one_command_of_its_own_plugin_while_its_call_executes() {
    let (harness, tool) = setup(false).await;
    tool.behaving(Behavior::TriesTheTicket);
    let model = &harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        START_ON_HOST,
        &on("build-box", "tail -f /var/log/syslog"),
    )]));
    model.push_reply(reply("Started."));

    let (conversation, run) = start(&harness, "Follow the syslog").await;
    harness.wait_for_run(&conversation, &run).await;
    let ticket = lock(&tool.seen).tickets[0].clone().unwrap();
    let (late, late_host) = FakeCommand::new();
    let after_the_call = tool.commands.start(ticket, Box::new(late)).await;

    {
        let seen = lock(&tool.seen);
        // Another plugin's service refused the ticket, the plugin's own took
        // it once, and the second start with it was refused.
        assert_eq!(
            seen.refusals,
            vec![
                BackgroundError::TicketInvalid,
                BackgroundError::TicketInvalid
            ]
        );
        assert_eq!(seen.hosts[0].stops(), 1);
        assert_eq!(seen.hosts[1].stops(), 0);
        assert_eq!(seen.hosts[2].stops(), 1);
    }
    assert_eq!(after_the_call, Err(BackgroundError::TicketInvalid));
    assert_eq!(late_host.stops(), 1);
    assert_eq!(harness.runtime.running_background_count(), 1);
    harness.runtime.shutdown().await;
}

#[tokio::test]
async fn a_command_in_the_foreground_gets_no_ticket() {
    let (harness, tool) = setup(false).await;
    tool.behaving(Behavior::Foreground);
    let model = &harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        START_ON_HOST,
        &on("build-box", "uptime"),
    )]));
    model.push_reply(reply("It has no ticket."));

    let (conversation, run) = start(&harness, "Check the uptime").await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    assert_eq!(lock(&tool.seen).tickets, vec![None]);
    assert!(transcript.background_commands.is_empty());
    harness.runtime.shutdown().await;
}

#[tokio::test]
async fn a_ninth_command_of_a_conversation_is_refused_and_ended() {
    let (harness, tool) = setup(false).await;
    let calls: Vec<(String, String)> = (1..=9)
        .map(|index| {
            (
                format!("call_{index}"),
                on("build-box", &format!("tail -f /var/log/app-{index}.log")),
            )
        })
        .collect();
    let calls: Vec<(&str, &str, &str)> = calls
        .iter()
        .map(|(id, arguments)| (id.as_str(), START_ON_HOST, arguments.as_str()))
        .collect();
    let model = &harness.model;
    model.push_reply(tool_calls(&calls));
    model.push_reply(reply("Eight run."));

    let (conversation, run) = start(&harness, "Follow every log").await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    {
        let seen = lock(&tool.seen);
        assert_eq!(
            seen.refusals,
            vec![BackgroundError::TooMany {
                scope: BackgroundScope::Conversation,
                limit: 8,
            }]
        );
        let ended = seen.hosts.iter().filter(|host| host.stops() == 1).count();
        assert_eq!(ended, 1);
    }
    assert_eq!(transcript.background_commands.len(), 8);
    harness.runtime.shutdown().await;
}

#[tokio::test]
async fn an_end_on_the_host_keeps_its_status_and_an_output_that_just_ends_lets_the_command_go() {
    let (harness, tool) = setup(false).await;
    let model = &harness.model;
    model.push_reply(tool_calls(&[
        ("call_1", START_ON_HOST, &on("build-box", "make test")),
        ("call_2", START_ON_HOST, &on("build-box", "make watch")),
    ]));
    model.push_reply(reply("Both run."));

    let (conversation, run) = start(&harness, "Test and watch").await;
    harness.wait_for_run(&conversation, &run).await;
    tool.host(0).write(&["ok 12 tests"]);
    tool.host(0).end(RemoteEnding::Exited { status: 3 });
    tool.host(1).let_go();
    ended(&harness, &conversation, 1).await;
    let transcript = ended(&harness, &conversation, 2).await;

    let tested = &transcript.background_commands[&1];
    assert_eq!(tested.ending, Some(BackgroundEnding::EndedByHost));
    assert_eq!(tested.exit_code, Some(3));
    assert_eq!(tested.lines, 1);
    let watched = &transcript.background_commands[&2];
    assert_eq!(watched.ending, Some(BackgroundEnding::LetGo));
    assert_eq!(watched.exit_code, None);
    // A command that ended on its own is not asked to stop.
    assert_eq!(tool.host(0).stops(), 0);
    assert_eq!(tool.host(1).stops(), 0);
    harness.runtime.shutdown().await;
}

#[tokio::test]
async fn an_end_on_the_host_wakes_an_idle_agent() {
    let (harness, tool) = setup(true).await;
    let model = &harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        START_ON_HOST,
        &on("build-box", "make deploy"),
    )]));
    model.push_reply(reply("The deploy runs."));
    model.push_reply(reply("The deploy failed with code 2."));

    let (conversation, run) = start(&harness, "Deploy").await;
    harness.wait_for_run(&conversation, &run).await;
    tool.host(0).end(RemoteEnding::Exited { status: 2 });
    let transcript =
        harness
            .wait_until(&conversation, |transcript| {
                transcript.turns.len() == 2
                    && transcript.turns.values().last().is_some_and(|turn| {
                        turn.run.as_ref().is_some_and(|run| !run.state.is_active())
                    })
            })
            .await;

    assert_eq!(transcript.turns.len(), 2);
    let requests = model.reply_requests();
    assert_eq!(requests.len(), 3);
    harness.runtime.shutdown().await;
}

#[tokio::test]
async fn turning_the_plugin_off_ends_its_commands() {
    let (harness, tool) = setup(false).await;
    let model = &harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        START_ON_HOST,
        &on("build-box", "journalctl -f"),
    )]));
    model.push_reply(reply("It runs."));

    let (conversation, run) = start(&harness, "Follow the journal").await;
    harness.wait_for_run(&conversation, &run).await;
    let registered = turn_off_plugin(&harness.runtime, PLUGIN, Duration::from_secs(1)).await;
    let transcript = ended(&harness, &conversation, 1).await;

    assert!(registered);
    assert_eq!(
        transcript.background_commands[&1].ending,
        Some(BackgroundEnding::PluginOff)
    );
    assert_eq!(tool.host(0).stops(), 1);
    assert_eq!(
        work(&harness),
        vec![WorkItem::new(BACKGROUND_COMMANDS_WORK, 0)]
    );
    harness.runtime.shutdown().await;
}

#[tokio::test]
async fn quitting_ends_a_plugin_s_commands_as_interrupted() {
    let (harness, tool) = setup(false).await;
    let model = &harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        START_ON_HOST,
        &on("build-box", "journalctl -f"),
    )]));
    model.push_reply(reply("It runs."));

    let (conversation, run) = start(&harness, "Follow the journal").await;
    harness.wait_for_run(&conversation, &run).await;
    harness.runtime.shutdown().await;

    assert_eq!(tool.host(0).stops(), 1);
    let stored = harness
        .store
        .background_command(&conversation, 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.document.ending, Some(BackgroundEnding::Interrupted));
}
