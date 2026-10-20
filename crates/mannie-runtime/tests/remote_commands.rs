//! Commands on remote hosts through a plugin's tool with side effects
//! (ADR-0036; ADR-0034, decisions 3 and 8): the policy asks, grants and rules
//! cover one host at one endpoint, the tool executes exactly what was
//! authorized and archives what does not fit, a tool that breaks the SDK's
//! rules runs nothing, and Stop or turning the plugin off leaves a running
//! operation uncertain while the reply goes on.

#![cfg(test)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::FutureExt;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::points::tools::{
    AgentSelector, PluginTool, RemoteCommandAccess, Tool, ToolAccess, ToolArchive, ToolCall,
    ToolExecution, ToolRefusal, ToolResult, ToolSpec, Tools,
};
use mannie_runtime::testing::{
    Harness, HarnessOptions, SeenPart, Transcript, add_plugin, reply, tool_calls, turn_off_plugin,
};
use mannie_tools::{MIN_RESULT_LIMIT_BYTES, RESULT_LIMIT_BYTES};
use mannie_types::chat::StartConversation;
use mannie_types::ids::{ConversationId, PartId, RunId};
use mannie_types::run::RunState;
use mannie_types::tools::{
    ApprovalChoice, ApprovalReason, ApprovalRequest, DecideApproval, Decision, GrantScope,
    Interruption, NotRunReason, ReadToolOutput, RemoteTarget, ToolCallStatus, ToolFailure,
    ToolOutcome, UpdateToolsSettings,
};
use mannie_types::transcript::PartContent;
use serde_json::{Value, json};

/// The fake plugin's ID.
const PLUGIN: &str = "shell";

/// The fake plugin's tool.
const RUN_ON_HOST: &str = "run_on_host";

/// The endpoint of every host of the fake plugin.
const ENDPOINT: &str = "deploy@build-box.lan:22";

/// The pinned key of every host of the fake plugin.
const IDENTITY: &str = "SHA256:vh3fjoyg7pBnxLhORIqZjyJQqYYN/gzBTuj+mPdeJlo";

/// The plugin's key for a host, by its name.
fn target_of(host: &str) -> String {
    format!("key-of-{host}")
}

/// What a call of the fake tool does when it executes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Behavior {
    /// It answers at once, with its output archived as `stdout`.
    #[default]
    Answers,
    /// It never answers, until its future is dropped.
    Hangs,
}

/// A plugin's tool that runs a command on a remote host. Its preparation
/// returns the remote command, or nothing at all when `silent`, and it keeps
/// what it executed.
#[derive(Debug, Default)]
struct RemoteShell {
    behavior: Mutex<Behavior>,
    silent: AtomicBool,
    executions: Mutex<Vec<ToolExecution>>,
    started: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
}

/// Sets its flag when it is dropped.
#[derive(Debug)]
struct SetOnDrop(Arc<AtomicBool>);

impl Drop for SetOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl RemoteShell {
    fn hanging() -> Self {
        let shell = Self::default();
        *shell.behavior.lock().unwrap() = Behavior::Hangs;
        shell
    }

    fn executions(&self) -> Vec<ToolExecution> {
        self.executions.lock().unwrap().clone()
    }
}

impl Tool for RemoteShell {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            RUN_ON_HOST,
            "Runs a command on a host.",
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
        let silent = self.silent.load(Ordering::SeqCst);
        async move {
            let arguments: Value = serde_json::from_str(&call.arguments)
                .map_err(|error| ToolRefusal::new(error.to_string()))?;
            if silent {
                return Ok(Vec::new());
            }
            let host = arguments["host"].as_str().unwrap_or_default().to_owned();
            let command = arguments["command"].as_str().unwrap_or_default().to_owned();
            Ok(vec![ToolAccess::RemoteCommand(RemoteCommandAccess::new(
                target_of(&host),
                host,
                ENDPOINT.to_owned(),
                IDENTITY.to_owned(),
                command,
            ))])
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let behavior = *self.behavior.lock().unwrap();
        let command = execution
            .authorized
            .iter()
            .filter_map(|access| match access {
                ToolAccess::RemoteCommand(remote) => Some(remote.command.clone()),
                ToolAccess::Path(_) | ToolAccess::Program(_) => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        let archive_id = execution.archive_ids.first().cloned();
        self.executions.lock().unwrap().push(execution);
        let started = Arc::clone(&self.started);
        let guard = SetOnDrop(Arc::clone(&self.dropped));
        async move {
            started.store(true, Ordering::SeqCst);
            match behavior {
                Behavior::Answers => {
                    let archives = archive_id
                        .map(|id| ToolArchive::new(id, "stdout", format!("ran: {command}\n")))
                        .into_iter()
                        .collect();
                    drop(guard);
                    ToolResult::new(format!("Ran {command}.")).with_archives(archives)
                }
                Behavior::Hangs => {
                    let never = futures_util::future::pending::<ToolResult>().await;
                    drop(guard);
                    never
                }
            }
        }
        .boxed()
    }
}

/// The fake plugin with its tool, which has side effects and is offered to
/// the owner's conversations.
fn with_shell(shell: &Arc<RemoteShell>) -> Contributions {
    let tool: Arc<dyn Tool> = Arc::<RemoteShell>::clone(shell);
    Contributions::new()
        .add::<Tools>(PluginTool::new(tool, AgentSelector::Conversations).with_side_effects())
}

/// The arguments of a call of the fake tool.
fn on(host: &str, command: &str) -> String {
    json!({ "activity": "Running", "host": host, "command": command }).to_string()
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

/// Waits until a call waits for the owner, answers it with `choice`, and
/// waits until the call no longer waits, so the next answer finds the next
/// call.
async fn answer(harness: &Harness, conversation: &ConversationId, choice: ApprovalChoice) {
    let transcript = harness
        .wait_until(conversation, |transcript| {
            waiting_call(transcript).is_some()
        })
        .await;
    let (part_id, request) = waiting_call(&transcript).unwrap();
    harness
        .runtime
        .decide_approval(DecideApproval {
            conversation_id: conversation.clone(),
            part_id: part_id.clone(),
            digest: request.digest,
            choice,
        })
        .await
        .unwrap();
    harness
        .wait_until(conversation, |transcript| {
            waiting_call(transcript).is_none_or(|(waiting, _request)| waiting != part_id)
        })
        .await;
}

/// Waits until the tool runs, as its execution notes.
async fn wait_until_started(shell: &RemoteShell) {
    let started = Arc::clone(&shell.started);
    tokio::time::timeout(Duration::from_secs(20), async move {
        while !started.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the execution starts");
}

#[tokio::test]
async fn a_remote_command_asks_and_runs_what_was_authorized_once_allowed() {
    let harness = harness().await;
    let shell = Arc::new(RemoteShell::default());
    add_plugin(&harness.runtime, PLUGIN, with_shell(&shell)).unwrap();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        RUN_ON_HOST,
        &on("build-box", "systemctl status nginx"),
    )]));
    harness.model.push_reply(reply("It runs."));

    let (conversation, run) = start(&harness, "OWNER: check nginx").await;
    let transcript = harness
        .wait_until(&conversation, |transcript| {
            waiting_call(transcript).is_some()
        })
        .await;
    let (_part, request) = waiting_call(&transcript).unwrap();
    answer(&harness, &conversation, ApprovalChoice::AllowOnce).await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    assert_eq!(request.reasons, vec![ApprovalReason::RemoteCommand]);
    assert_eq!(
        request.grants,
        vec![GrantScope::RemoteCommand {
            plugin: PLUGIN.to_owned(),
            target: target_of("build-box"),
            label: "build-box".to_owned(),
            endpoint: ENDPOINT.to_owned(),
            command: "systemctl status nginx".to_owned(),
        }]
    );
    let proposal = request.rule.expect("a rule can cover the command");
    assert_eq!(
        proposal.remote,
        Some(RemoteTarget {
            plugin: PLUGIN.to_owned(),
            target: target_of("build-box"),
            label: "build-box".to_owned(),
            endpoint: ENDPOINT.to_owned(),
        })
    );
    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(call.decision, Some(Decision::AllowedOnce));
    let result = call.result.expect("the call ran");
    assert_eq!(
        result.outcome,
        ToolOutcome::Plugin {
            plugin: PLUGIN.to_owned(),
            data: None,
            failure: None,
        }
    );
    assert_eq!(result.output, "Ran systemctl status nginx.");
    // The tool executed exactly the authorized access, within the run's
    // result limit and with archive IDs of its own.
    let [execution] = shell.executions().try_into().unwrap();
    assert_eq!(execution.authorized.len(), 1);
    let limit = usize::try_from(execution.result_limit).unwrap();
    assert!(
        (MIN_RESULT_LIMIT_BYTES..=RESULT_LIMIT_BYTES).contains(&limit),
        "{limit}"
    );
    assert_eq!(execution.archive_ids.len(), 4);
    let [archive] = result.archives.try_into().unwrap();
    assert_eq!(archive.name, "stdout");
    assert_eq!(archive.id, execution.archive_ids[0]);
    let page = harness
        .runtime
        .tool_output(ReadToolOutput {
            conversation_id: conversation.clone(),
            archive_id: archive.id,
            first_line: 1,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(page.lines, vec!["ran: systemctl status nginx".to_owned()]);
}

#[tokio::test]
async fn a_grant_for_the_conversation_covers_the_same_command_on_the_same_host_only() {
    let harness = harness().await;
    let shell = Arc::new(RemoteShell::default());
    add_plugin(&harness.runtime, PLUGIN, with_shell(&shell)).unwrap();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        RUN_ON_HOST,
        &on("build-box", "uptime"),
    )]));
    harness.model.push_reply(tool_calls(&[(
        "call_2",
        RUN_ON_HOST,
        &on("build-box", "uptime"),
    )]));
    harness.model.push_reply(tool_calls(&[(
        "call_3",
        RUN_ON_HOST,
        &on("web-01", "uptime"),
    )]));
    harness.model.push_reply(reply("Done."));

    let (conversation, run) = start(&harness, "OWNER: uptime everywhere").await;
    answer(
        &harness,
        &conversation,
        ApprovalChoice::AllowForConversation,
    )
    .await;
    answer(&harness, &conversation, ApprovalChoice::Deny).await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let calls = transcript.tool_calls(0);
    let decisions: Vec<_> = calls.iter().map(|call| call.decision).collect();
    assert_eq!(
        decisions,
        vec![
            Some(Decision::AllowedForConversation),
            Some(Decision::Grant),
            Some(Decision::Denied),
        ]
    );
    assert_eq!(shell.executions().len(), 2);
}

#[tokio::test]
async fn a_rule_saved_for_a_host_allows_its_leading_words_there() {
    let harness = harness().await;
    let shell = Arc::new(RemoteShell::default());
    add_plugin(&harness.runtime, PLUGIN, with_shell(&shell)).unwrap();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        RUN_ON_HOST,
        &on("build-box", "systemctl status nginx"),
    )]));
    harness.model.push_reply(tool_calls(&[(
        "call_2",
        RUN_ON_HOST,
        &on("build-box", "systemctl status sshd"),
    )]));
    harness.model.push_reply(reply("Both are fine."));

    let (conversation, run) = start(&harness, "OWNER: check the services").await;
    answer(
        &harness,
        &conversation,
        ApprovalChoice::SaveRule {
            prefix_words: 2,
            limit_to_folder: true,
        },
    )
    .await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let decisions: Vec<_> = transcript
        .tool_calls(0)
        .iter()
        .map(|call| call.decision)
        .collect();
    assert_eq!(
        decisions,
        vec![Some(Decision::RuleSaved), Some(Decision::Rule)]
    );
    let [rule] = harness
        .runtime
        .remote_command_rules()
        .await
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(
        rule.words,
        vec!["systemctl".to_owned(), "status".to_owned()]
    );
    assert_eq!(rule.target.target, target_of("build-box"));
    assert_eq!(harness.runtime.command_rules().await.unwrap(), Vec::new());
}

#[tokio::test]
async fn full_access_runs_a_remote_command_without_asking() {
    let harness = harness().await;
    let shell = Arc::new(RemoteShell::default());
    add_plugin(&harness.runtime, PLUGIN, with_shell(&shell)).unwrap();
    harness
        .runtime
        .update_tools_settings(UpdateToolsSettings {
            allow_changes_in_workspace: false,
            command_environment: Vec::new(),
            full_access_for_new_conversations: true,
        })
        .await
        .unwrap();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        RUN_ON_HOST,
        &on("build-box", "reboot; rm -rf /tmp/x"),
    )]));
    harness.model.push_reply(reply("Rebooted."));

    let (conversation, run) = start(&harness, "OWNER: reboot it").await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.decision, Some(Decision::FullAccess));
    assert_eq!(call.status, ToolCallStatus::Completed);
}

#[tokio::test]
async fn a_tool_with_side_effects_that_names_nothing_it_touches_runs_nothing() {
    let harness = harness().await;
    let shell = Arc::new(RemoteShell::default());
    shell.silent.store(true, Ordering::SeqCst);
    add_plugin(&harness.runtime, PLUGIN, with_shell(&shell)).unwrap();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        RUN_ON_HOST,
        &on("build-box", "uptime"),
    )]));
    harness.model.push_reply(reply("It failed."));

    let (conversation, run) = start(&harness, "OWNER: uptime").await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(
        call.result.map(|result| result.outcome),
        Some(ToolOutcome::Failed {
            failure: ToolFailure::PluginInvalid {
                plugin: PLUGIN.to_owned()
            }
        })
    );
    assert_eq!(shell.executions(), Vec::new());
}

#[tokio::test]
async fn turning_the_plugin_off_while_a_call_waits_for_the_owner_runs_nothing() {
    let harness = harness().await;
    let shell = Arc::new(RemoteShell::default());
    add_plugin(&harness.runtime, PLUGIN, with_shell(&shell)).unwrap();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        RUN_ON_HOST,
        &on("build-box", "uptime"),
    )]));
    harness.model.push_reply(reply("The plugin is off."));

    let (conversation, run) = start(&harness, "OWNER: uptime").await;
    harness
        .wait_until(&conversation, |transcript| {
            waiting_call(transcript).is_some()
        })
        .await;
    assert!(turn_off_plugin(&harness.runtime, PLUGIN, Duration::from_secs(5)).await);
    answer(&harness, &conversation, ApprovalChoice::AllowOnce).await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::NotRun);
    assert_eq!(
        call.not_run,
        Some(NotRunReason::PluginOff {
            plugin: PLUGIN.to_owned()
        })
    );
    assert_eq!(shell.executions(), Vec::new());
}

#[tokio::test]
async fn turning_the_plugin_off_makes_a_running_operation_uncertain_and_the_reply_goes_on() {
    let harness = harness().await;
    let shell = Arc::new(RemoteShell::hanging());
    add_plugin(&harness.runtime, PLUGIN, with_shell(&shell)).unwrap();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        RUN_ON_HOST,
        &on("build-box", "sleep 600"),
    )]));
    harness.model.push_reply(reply("The plugin went off."));

    let (conversation, run) = start(&harness, "OWNER: wait").await;
    answer(&harness, &conversation, ApprovalChoice::AllowOnce).await;
    wait_until_started(&shell).await;
    assert!(turn_off_plugin(&harness.runtime, PLUGIN, Duration::from_millis(50)).await);
    let transcript = harness.wait_for_run(&conversation, &run).await;

    assert!(shell.dropped.load(Ordering::SeqCst));
    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Uncertain);
    assert_eq!(
        call.interruption,
        Some(Interruption::PluginOff {
            plugin: PLUGIN.to_owned()
        })
    );
}

#[tokio::test]
async fn stopping_a_running_operation_of_a_plugin_leaves_it_uncertain() {
    let harness = harness().await;
    let shell = Arc::new(RemoteShell::hanging());
    add_plugin(&harness.runtime, PLUGIN, with_shell(&shell)).unwrap();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        RUN_ON_HOST,
        &on("build-box", "sleep 600"),
    )]));

    let (conversation, run) = start(&harness, "OWNER: wait").await;
    answer(&harness, &conversation, ApprovalChoice::AllowOnce).await;
    wait_until_started(&shell).await;
    harness.runtime.stop_reply(&conversation).await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    assert!(shell.dropped.load(Ordering::SeqCst));
    assert_eq!(transcript.run_state(&run), Some(RunState::Stopped));
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Uncertain);
    assert_eq!(call.interruption, Some(Interruption::Stopped));
    assert_eq!(call.result, None);
}

#[tokio::test]
async fn turning_the_plugin_off_waits_for_an_operation_that_ends_in_time() {
    let harness = harness().await;
    let shell = Arc::new(RemoteShell::default());
    add_plugin(&harness.runtime, PLUGIN, with_shell(&shell)).unwrap();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        RUN_ON_HOST,
        &on("build-box", "uptime"),
    )]));
    harness.model.push_reply(reply("Done."));

    let (conversation, run) = start(&harness, "OWNER: uptime").await;
    answer(&harness, &conversation, ApprovalChoice::AllowOnce).await;
    let transcript = harness.wait_for_run(&conversation, &run).await;
    assert!(turn_off_plugin(&harness.runtime, PLUGIN, Duration::from_secs(5)).await);

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(call.interruption, None);
}
