//! Plugins' tools that run installed programs (ADR-0038, decisions 5 and 6):
//! a call's program access gets the program's path and the arguments that
//! the plugin's manifest fixes, a run with an effect beyond the workspace
//! asks even where changes run without asking, "Allow for this
//! conversation" covers the same run, a saved rule covers runs that start
//! with its words and have no other effect, full access runs everything, and
//! a program that the manifest does not declare breaks the SDK's rules.
//!
//! The tests find the installed git, which the plugin Git needs.

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures_util::FutureExt;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::HostProfile;
use mannie_plugin_sdk::contracts::{AccessMode, ProgramEffect};
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::host::PluginBinding;
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::manifest::Manifest;
use mannie_plugin_sdk::points::tools::{
    AgentSelector, PathAccess, PluginTool, ProgramAccess, Tool, ToolAccess, ToolCall,
    ToolExecution, ToolRefusal, ToolResult, ToolSpec, Tools,
};
use mannie_plugin_sdk::services::processes::{Processes, ProcessesApi};
use mannie_runtime::testing::{
    Harness, HarnessOptions, SeenPart, Transcript, add_plugin, reply, tool_calls,
};
use mannie_types::chat::StartConversation;
use mannie_types::ids::{ConversationId, PartId, ProjectId, RunId};
use mannie_types::tools::{
    Access, ApprovalChoice, ApprovalReason, ApprovalRequest, DecideApproval, Decision,
    ProgramEffectKey, SetFullAccess, ToolCallStatus, ToolFailure, ToolOutcome, UpdateToolsSettings,
};
use mannie_types::transcript::PartContent;
use serde_json::{Value, json};
use tempfile::TempDir;

/// The fake plugin's ID.
const PLUGIN: &str = "history";

/// The plugin's tool, which commits with git.
const COMMIT: &str = "history_commit";

/// The state that the tool read of the repository.
const STATE: &str = "HEAD 1a2b3c";

/// A tool that runs a program of the call's arguments with the hooks that
/// they name, beside a write of the workspace, and keeps its executions.
#[derive(Debug, Default)]
struct CommitTool {
    executions: Mutex<Vec<ToolExecution>>,
}

impl CommitTool {
    fn executions(&self) -> Vec<ToolExecution> {
        self.executions.lock().unwrap().clone()
    }
}

/// The effect of a hook of the repository.
fn hook(name: &str) -> ProgramEffect {
    ProgramEffect::RunsFolderCode {
        name: name.to_owned(),
        source: format!(".git/hooks/{name}"),
    }
}

impl Tool for CommitTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            COMMIT,
            "Commits.",
            json!({
                "additionalProperties": false,
                "properties": {
                    "program": { "type": "string" },
                    "arguments": { "type": "array", "items": { "type": "string" } },
                    "hooks": { "type": "array", "items": { "type": "string" } },
                },
                "required": ["program", "arguments", "hooks"],
                "type": "object",
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        async move {
            let arguments: Value = serde_json::from_str(&call.arguments)
                .map_err(|error| ToolRefusal::new(error.to_string()))?;
            let workspace = call
                .workspace
                .ok_or_else(|| ToolRefusal::new("The conversation has no workspace."))?;
            let texts = |key: &str| -> Vec<String> {
                arguments[key]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            };
            let program = arguments["program"].as_str().unwrap_or_default();
            let mut run =
                ProgramAccess::new(program, &workspace, texts("arguments")).with_state(STATE);
            for name in texts("hooks") {
                run = run.with_effect(hook(&name));
            }
            Ok(vec![
                ToolAccess::Path(PathAccess::new(&workspace, AccessMode::Write)),
                ToolAccess::Program(run),
            ])
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        self.executions.lock().unwrap().push(execution);
        async { ToolResult::new("Committed.") }.boxed()
    }
}

/// The arguments of a call of the tool.
fn commit(program: &str, arguments: &[&str], hooks: &[&str]) -> String {
    json!({
        "activity": "Committing",
        "program": program,
        "arguments": arguments,
        "hooks": hooks,
    })
    .to_string()
}

/// A runtime whose changes in the workspace run without asking, a project
/// whose folder is the workspace, the plugin with its tool, and its service
/// `processes`, whose binding records the programs of its manifest.
struct Setup {
    harness: Harness,
    _workspace: TempDir,
    root: PathBuf,
    project: ProjectId,
    tool: Arc<CommitTool>,
    processes: Arc<dyn ProcessesApi>,
}

impl Setup {
    async fn new() -> Self {
        let harness = Harness::with_options(HarnessOptions {
            use_tools: true,
            ..HarnessOptions::default()
        })
        .await;
        harness
            .runtime
            .update_tools_settings(UpdateToolsSettings {
                allow_changes_in_workspace: true,
                command_environment: Vec::new(),
                full_access_for_new_conversations: false,
                wake_on_command_end: true,
            })
            .await
            .unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let project = harness
            .runtime
            .add_project(workspace.path().to_str().unwrap().to_owned())
            .await
            .unwrap();
        let processes = bind(&harness);
        let tool = Arc::new(CommitTool::default());
        let entry: Arc<dyn Tool> = Arc::<CommitTool>::clone(&tool);
        add_plugin(
            &harness.runtime,
            PLUGIN,
            Contributions::new().add::<Tools>(
                PluginTool::new(entry, AgentSelector::Conversations).with_side_effects(),
            ),
        )
        .unwrap();
        Self {
            harness,
            _workspace: workspace,
            root: PathBuf::from(&project.folder),
            project: project.id,
            tool,
            processes,
        }
    }

    async fn start(&self, text: &str) -> (ConversationId, RunId) {
        let started = self
            .harness
            .runtime
            .start_conversation(
                StartConversation::new(text).with_project(Some(self.project.clone())),
            )
            .await
            .unwrap();
        (started.conversation_id, started.run_id)
    }

    /// The installed git, as the core found it.
    async fn git(&self) -> String {
        let path = self.processes.locate("git").await.unwrap();
        path.to_str().unwrap().to_owned()
    }
}

/// The manifest of the plugin, which runs git with a fixed setting.
fn manifest() -> Manifest {
    serde_json::from_value(json!({
        "id": PLUGIN,
        "sdk": 1,
        "order": 40,
        "rust": { "entry": "mannie-history" },
        "services": { "processes": 1 },
        "processes": {
            "git": { "reason": "Commits.", "fixed": ["-c", "gc.auto=0"] },
        },
    }))
    .unwrap()
}

/// Binds the runtime's service `processes` to the plugin, as the host does
/// when the plugin starts.
fn bind(harness: &Harness) -> Arc<dyn ProcessesApi> {
    let id = PluginId::parse(PLUGIN).unwrap();
    let manifest = manifest();
    let binding = PluginBinding::new(&id, &manifest, HostProfile::Development);
    let provider = harness
        .runtime
        .service_providers()
        .into_iter()
        .find(|provider| provider.name() == "processes")
        .expect("the runtime provides the service processes");
    provider
        .bind(&binding)
        .unwrap()
        .get::<Processes>()
        .expect("the bound service is processes")
}

/// The tool call that waits for the owner, if one does.
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

/// Waits until a call other than `answered` waits for the owner, answers it
/// with `choice`, and returns its part and its request.
async fn answer(
    harness: &Harness,
    conversation: &ConversationId,
    answered: Option<&PartId>,
    choice: ApprovalChoice,
) -> (PartId, ApprovalRequest) {
    let waiting = |transcript: &Transcript| {
        waiting_call(transcript).filter(|(part_id, _request)| Some(part_id) != answered)
    };
    let transcript = harness
        .wait_until(conversation, |transcript| waiting(transcript).is_some())
        .await;
    let (part_id, request) = waiting(&transcript).unwrap();
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
    (part_id, request)
}

/// The program access of an execution.
fn program_of(execution: &ToolExecution) -> ProgramAccess {
    execution
        .authorized
        .iter()
        .find_map(|access| match access {
            ToolAccess::Program(run) => Some(run.clone()),
            ToolAccess::Path(_) | ToolAccess::RemoteCommand(_) => None,
        })
        .expect("the execution names its program's run")
}

#[tokio::test]
async fn a_run_with_a_hook_asks_and_allowing_it_for_the_conversation_covers_the_same_run() {
    let setup = Setup::new().await;
    let arguments = commit("git", &["commit", "-F", "-"], &["pre-commit"]);
    setup.harness.model.push_reply(tool_calls(&[
        ("call_1", COMMIT, &arguments),
        ("call_2", COMMIT, &arguments),
    ]));
    setup.harness.model.push_reply(reply("Committed twice."));

    let (conversation, run) = setup.start("OWNER: commit").await;
    let (_part, request) = answer(
        &setup.harness,
        &conversation,
        None,
        ApprovalChoice::AllowForConversation,
    )
    .await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    // The write of the workspace runs; only the hook asks.
    assert_eq!(request.reasons, [ApprovalReason::Program]);
    let rule = request.rule.expect("a rule can cover the run");
    assert_eq!(rule.words, ["git", "commit", "-F", "-"]);
    let target = rule.program.expect("the rule names the program");
    assert_eq!(target.plugin, PLUGIN);
    assert_eq!(target.program, setup.git().await);
    assert_eq!(
        target.effects,
        [ProgramEffectKey::RunsFolderCode {
            name: "pre-commit".to_owned(),
            source: ".git/hooks/pre-commit".to_owned(),
        }]
    );
    let [first, second] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(first.decision, Some(Decision::AllowedForConversation));
    assert_eq!(second.decision, Some(Decision::Grant));
    assert_eq!(second.status, ToolCallStatus::Completed);
    let Some(Access::Program {
        plugin,
        program,
        fixed,
        working_directory,
        state,
        ..
    }) = first
        .accesses
        .iter()
        .find(|access| matches!(access, Access::Program { .. }))
    else {
        panic!("the call names its program's run: {:?}", first.accesses);
    };
    assert_eq!(plugin, PLUGIN);
    assert_eq!(*program, setup.git().await);
    assert_eq!(fixed, &["-c", "gc.auto=0"]);
    assert_eq!(Path::new(working_directory), setup.root);
    assert_eq!(state.as_deref(), Some(STATE));
    // The tool learned exactly the run that the owner allowed.
    let executions = setup.tool.executions();
    assert_eq!(executions.len(), 2);
    let run = program_of(&executions[0]);
    assert_eq!(run.arguments, ["commit", "-F", "-"]);
    assert_eq!(run.working_directory, setup.root);
    assert_eq!(run.effects, [hook("pre-commit")]);
}

#[tokio::test]
async fn a_saved_rule_allows_runs_that_start_with_its_words_without_a_new_effect() {
    let setup = Setup::new().await;
    setup.harness.model.push_reply(tool_calls(&[
        (
            "call_1",
            COMMIT,
            &commit("git", &["commit", "-F", "-"], &["pre-commit"]),
        ),
        (
            "call_2",
            COMMIT,
            &commit("git", &["commit", "--amend", "-F", "-"], &["pre-commit"]),
        ),
        (
            "call_3",
            COMMIT,
            &commit("git", &["commit", "-F", "-"], &["pre-commit", "commit-msg"]),
        ),
    ]));
    setup.harness.model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("OWNER: commit").await;
    let (first, _request) = answer(
        &setup.harness,
        &conversation,
        None,
        ApprovalChoice::SaveRule {
            prefix_words: 2,
            limit_to_folder: false,
        },
    )
    .await;
    let (_part, new_hook) = answer(
        &setup.harness,
        &conversation,
        Some(&first),
        ApprovalChoice::AllowOnce,
    )
    .await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let [saved, amended, with_new_hook] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(saved.decision, Some(Decision::RuleSaved));
    assert_eq!(amended.decision, Some(Decision::Rule));
    assert_eq!(with_new_hook.decision, Some(Decision::AllowedOnce));
    assert_eq!(new_hook.reasons, [ApprovalReason::Program]);
    let [rule] = setup
        .harness
        .store
        .program_rules()
        .await
        .unwrap()
        .try_into()
        .unwrap();
    assert_eq!(rule.words, ["git", "commit"]);
    assert_eq!(rule.folder, None);
    assert_eq!(rule.target.name, "git");
}

#[tokio::test]
async fn full_access_runs_a_program_without_asking() {
    let setup = Setup::new().await;
    setup.harness.model.push_reply(reply("Ready."));
    setup.harness.model.push_reply(tool_calls(&[(
        "call_1",
        COMMIT,
        &commit("git", &["push", "--force-with-lease"], &["pre-push"]),
    )]));
    setup.harness.model.push_reply(reply("Pushed."));
    let (conversation, run) = setup.start("OWNER: get ready").await;
    setup.harness.wait_for_run(&conversation, &run).await;
    setup
        .harness
        .runtime
        .set_full_access(SetFullAccess {
            conversation_id: conversation.clone(),
            enabled: true,
        })
        .await
        .unwrap();

    let next = setup
        .harness
        .runtime
        .send_message(mannie_types::chat::SendMessage::new(
            conversation.clone(),
            "OWNER: push",
        ))
        .await
        .unwrap();
    let transcript = setup
        .harness
        .wait_for_run(&conversation, &next.run_id)
        .await;

    let [call] = transcript.tool_calls(1).try_into().unwrap();
    assert_eq!(call.decision, Some(Decision::FullAccess));
    assert_eq!(call.status, ToolCallStatus::Completed);
}

#[tokio::test]
async fn a_program_that_the_manifest_does_not_declare_breaks_the_sdk_s_rules() {
    let setup = Setup::new().await;
    setup.harness.model.push_reply(tool_calls(&[(
        "call_1",
        COMMIT,
        &commit("pandoc", &["notes.md"], &[]),
    )]));
    setup.harness.model.push_reply(reply("It failed."));

    let (conversation, run) = setup.start("OWNER: convert").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    let result = call.result.expect("the call ended with a result");
    assert_eq!(
        result.outcome,
        ToolOutcome::Failed {
            failure: ToolFailure::PluginInvalid {
                plugin: PLUGIN.to_owned()
            }
        }
    );
    assert_eq!(setup.tool.executions(), Vec::new());
}
