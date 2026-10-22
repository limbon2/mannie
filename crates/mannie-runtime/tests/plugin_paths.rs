//! Plugins' tools on the owner's paths (ADR-0038, decisions 1 to 4 and 9;
//! ADR-0039, decisions 2 and 3): a tool learns the conversation's workspace;
//! its reads run without asking inside the workspace and ask outside it; its
//! writes follow the setting for changes in the workspace; a changed state
//! sends the call back to the owner; it is offered where its marker lies; a
//! plugin's section learns the tools that a call is offered; and a call's
//! record keeps its plugin, its card's data and its failure code.

#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures_util::FutureExt;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::AccessMode;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::points::sections::{
    Section, SectionBody, SectionContext, SectionSource, Sections,
};
use mannie_plugin_sdk::points::tools::{
    AgentSelector, PathAccess, PluginTool, Tool, ToolAccess, ToolCall, ToolExecution, ToolOffering,
    ToolRefusal, ToolResult, ToolSpec, Tools,
};
use mannie_provider::ModelRequest;
use mannie_runtime::testing::{
    Harness, HarnessOptions, SeenPart, Transcript, add_plugin, reply, tool_calls,
};
use mannie_types::chat::StartConversation;
use mannie_types::ids::{ConversationId, PartId, ProjectId, RunId};
use mannie_types::tools::{
    ApprovalChoice, ApprovalReason, ApprovalRequest, DecideApproval, Decision, ToolCallStatus,
    ToolOutcome, UpdateToolsSettings,
};
use mannie_types::transcript::PartContent;
use serde_json::{Value, json};
use tempfile::TempDir;

/// The fake plugin's ID.
const PLUGIN: &str = "files";

/// The tool that reads a path.
const READ_PATH: &str = "files_read";

/// The tool that writes a path.
const WRITE_PATH: &str = "files_write";

/// The tool that is offered only where the workspace holds [`MARKER`].
const MARKED: &str = "files_marked";

/// The entry that offers [`MARKED`].
const MARKER: &str = "mannie-test-marker";

/// A tool of the fake plugin that reads or writes the path that its
/// arguments name, with the state that they give, and keeps its calls and
/// executions. It refuses a call with `refuse` in its arguments, with that
/// code, and answers an execution with its result.
#[derive(Debug)]
struct PathTool {
    name: &'static str,
    mode: AccessMode,
    calls: Mutex<Vec<ToolCall>>,
    executions: Mutex<Vec<ToolExecution>>,
    result: ToolResult,
}

impl PathTool {
    fn new(name: &'static str, mode: AccessMode) -> Self {
        Self {
            name,
            mode,
            calls: Mutex::new(Vec::new()),
            executions: Mutex::new(Vec::new()),
            result: ToolResult::new("Done."),
        }
    }

    fn answering(mut self, result: ToolResult) -> Self {
        self.result = result;
        self
    }

    fn calls(&self) -> Vec<ToolCall> {
        self.calls.lock().unwrap().clone()
    }

    fn executions(&self) -> Vec<ToolExecution> {
        self.executions.lock().unwrap().clone()
    }
}

impl Tool for PathTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            self.name,
            "Uses a path.",
            json!({
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string" },
                    "state": { "type": ["string", "null"] },
                    "refuse": { "type": ["string", "null"] },
                },
                "required": ["path", "state", "refuse"],
                "type": "object",
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        self.calls.lock().unwrap().push(call.clone());
        let mode = self.mode;
        async move {
            let arguments: Value = serde_json::from_str(&call.arguments)
                .map_err(|error| ToolRefusal::new(error.to_string()))?;
            if let Some(code) = arguments["refuse"].as_str() {
                return Err(ToolRefusal::new("This folder is not a repository.").with_code(code));
            }
            let path = arguments["path"].as_str().unwrap_or_default();
            let access = PathAccess::new(path, mode);
            let access = match arguments["state"].as_str() {
                Some(state) => access.with_state(state),
                None => access,
            };
            Ok(vec![ToolAccess::Path(access)])
        }
        .boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        self.executions.lock().unwrap().push(execution);
        let result = self.result.clone();
        async move { result }.boxed()
    }
}

/// A section that keeps the tools of each call that it renders for.
#[derive(Debug, Default)]
struct Recording {
    offered: Mutex<Vec<Vec<String>>>,
}

impl SectionSource for Recording {
    fn render(&self, context: &SectionContext) -> Option<String> {
        self.offered
            .lock()
            .unwrap()
            .push(context.offered_tools.clone());
        context
            .offers_tool(READ_PATH)
            .then(|| "Files may be read with files_read.".to_owned())
    }
}

/// A runtime whose profile uses tools, a project whose folder is the
/// workspace, and a folder outside it.
struct Setup {
    harness: Harness,
    _workspace: TempDir,
    _outside: TempDir,
    root: PathBuf,
    outside: PathBuf,
    project: ProjectId,
}

impl Setup {
    async fn new() -> Self {
        let harness = Harness::with_options(HarnessOptions {
            use_tools: true,
            ..HarnessOptions::default()
        })
        .await;
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let project = harness
            .runtime
            .add_project(workspace.path().to_str().unwrap().to_owned())
            .await
            .unwrap();
        let outside_path = PathBuf::from(mannie_tools::canonical_folder(outside.path()).unwrap());
        Self {
            harness,
            _workspace: workspace,
            _outside: outside,
            root: PathBuf::from(&project.folder),
            outside: outside_path,
            project: project.id,
        }
    }

    async fn allow_changes_in_workspace(&self, allowed: bool) {
        self.harness
            .runtime
            .update_tools_settings(UpdateToolsSettings {
                allow_changes_in_workspace: allowed,
                command_environment: Vec::new(),
                full_access_for_new_conversations: false,
                wake_on_command_end: true,
            })
            .await
            .unwrap();
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
}

/// The plugin with the tools and the section that a test gives it.
fn plugin(tools: Vec<PluginTool>, section: Option<&Arc<Recording>>) -> Contributions {
    let mut contributions = Contributions::new();
    for tool in tools {
        contributions = contributions.add::<Tools>(tool);
    }
    if let Some(section) = section {
        let source: Arc<dyn SectionSource> = Arc::<Recording>::clone(section);
        contributions =
            contributions.add::<Sections>(Section::new("files", 330, SectionBody::Source(source)));
    }
    contributions
}

fn reader(tool: &Arc<PathTool>) -> PluginTool {
    let tool: Arc<dyn Tool> = Arc::<PathTool>::clone(tool);
    PluginTool::new(tool, AgentSelector::Conversations).with_file_reads()
}

fn writer(tool: &Arc<PathTool>) -> PluginTool {
    let tool: Arc<dyn Tool> = Arc::<PathTool>::clone(tool);
    PluginTool::new(tool, AgentSelector::Conversations).with_side_effects()
}

/// The arguments of a call of a path tool.
fn on(path: &Path, state: Option<&str>) -> String {
    json!({ "activity": "Using a path", "path": path, "state": state, "refuse": null }).to_string()
}

fn tool_names(request: &ModelRequest) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
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

/// Waits until a call waits for the owner, answers it once, and returns the
/// reasons of its request.
async fn allow_once(harness: &Harness, conversation: &ConversationId) -> Vec<ApprovalReason> {
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
            part_id,
            digest: request.digest,
            choice: ApprovalChoice::AllowOnce,
        })
        .await
        .unwrap();
    request.reasons
}

#[tokio::test]
async fn a_tool_that_reads_files_reads_the_workspace_without_asking_and_asks_outside_it() {
    let setup = Setup::new().await;
    let tool = Arc::new(PathTool::new(READ_PATH, AccessMode::Read));
    add_plugin(
        &setup.harness.runtime,
        PLUGIN,
        plugin(vec![reader(&tool)], None),
    )
    .unwrap();
    setup.harness.model.push_reply(tool_calls(&[
        ("call_1", READ_PATH, &on(&setup.root, None)),
        ("call_2", READ_PATH, &on(&setup.outside, None)),
    ]));
    setup.harness.model.push_reply(reply("Read both."));

    let (conversation, run) = setup.start("OWNER: read").await;
    let reasons = allow_once(&setup.harness, &conversation).await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(reasons, [ApprovalReason::OutsideWorkspace]);
    let [inside, outside] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(inside.status, ToolCallStatus::Completed);
    assert_eq!(inside.decision, Some(Decision::Workspace));
    assert_eq!(outside.status, ToolCallStatus::Completed);
    assert_eq!(outside.decision, Some(Decision::AllowedOnce));
    // Both calls name their plugin, and the tool learned the workspace in
    // its preparation and its execution.
    assert_eq!(inside.plugin.as_deref(), Some(PLUGIN));
    assert!(
        tool.calls()
            .iter()
            .all(|call| call.workspace.as_deref() == Some(setup.root.as_path()))
    );
    let executions = tool.executions();
    assert_eq!(executions.len(), 2);
    assert_eq!(
        executions[0].workspace.as_deref(),
        Some(setup.root.as_path())
    );
    assert_eq!(
        executions[0].authorized,
        [ToolAccess::Path(PathAccess::new(
            setup.root.to_str().unwrap(),
            AccessMode::Read
        ))]
    );
}

#[tokio::test]
async fn a_tool_s_write_in_the_workspace_follows_the_setting_for_changes() {
    let setup = Setup::new().await;
    let tool = Arc::new(PathTool::new(WRITE_PATH, AccessMode::Write));
    add_plugin(
        &setup.harness.runtime,
        PLUGIN,
        plugin(vec![writer(&tool)], None),
    )
    .unwrap();
    setup.harness.model.push_reply(tool_calls(&[(
        "call_1",
        WRITE_PATH,
        &on(&setup.root, Some("HEAD 1a")),
    )]));
    setup.harness.model.push_reply(reply("Asked."));
    setup.harness.model.push_reply(tool_calls(&[(
        "call_2",
        WRITE_PATH,
        &on(&setup.root, Some("HEAD 1a")),
    )]));
    setup.harness.model.push_reply(reply("Wrote."));

    let (asking, asking_run) = setup.start("OWNER: write while changes ask").await;
    let reasons = allow_once(&setup.harness, &asking).await;
    setup.harness.wait_for_run(&asking, &asking_run).await;
    setup.allow_changes_in_workspace(true).await;
    let (allowed, allowed_run) = setup.start("OWNER: write while changes run").await;
    let transcript = setup.harness.wait_for_run(&allowed, &allowed_run).await;

    assert_eq!(reasons, [ApprovalReason::ChangeInWorkspace]);
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(call.decision, Some(Decision::Workspace));
    // The tool executed exactly the authorized path, with its state.
    let executions = tool.executions();
    let last = executions.last().unwrap();
    assert_eq!(
        last.authorized,
        [ToolAccess::Path(
            PathAccess::new(setup.root.to_str().unwrap(), AccessMode::Write).with_state("HEAD 1a")
        )]
    );
}

#[tokio::test]
async fn a_card_s_data_and_failure_reach_the_stored_outcome_and_a_coded_refusal_completes() {
    let setup = Setup::new().await;
    let tool = Arc::new(
        PathTool::new(READ_PATH, AccessMode::Read).answering(
            ToolResult::new("2 files conflict.")
                .with_data(json!({ "conflicted": ["a.txt", "b.txt"] }))
                .with_failure("merge_conflicts"),
        ),
    );
    add_plugin(
        &setup.harness.runtime,
        PLUGIN,
        plugin(vec![reader(&tool)], None),
    )
    .unwrap();
    let refusing = json!({
        "activity": "Using a path",
        "path": setup.root,
        "state": null,
        "refuse": "not_a_repository",
    })
    .to_string();
    setup.harness.model.push_reply(tool_calls(&[
        ("call_1", READ_PATH, &on(&setup.root, None)),
        ("call_2", READ_PATH, &refusing),
    ]));
    setup.harness.model.push_reply(reply("Done."));

    let (conversation, run) = setup.start("OWNER: check").await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let [described, refused] = transcript.tool_calls(0).try_into().unwrap();
    let result = described.result.unwrap();
    assert_eq!(
        result.outcome,
        ToolOutcome::Plugin {
            plugin: PLUGIN.to_owned(),
            data: Some(r#"{"conflicted":["a.txt","b.txt"]}"#.to_owned()),
            failure: Some("merge_conflicts".to_owned()),
        }
    );
    assert_eq!(result.output, "2 files conflict.");
    assert_eq!(refused.status, ToolCallStatus::Completed);
    let refusal = refused.result.unwrap();
    assert_eq!(
        refusal.outcome,
        ToolOutcome::Plugin {
            plugin: PLUGIN.to_owned(),
            data: None,
            failure: Some("not_a_repository".to_owned()),
        }
    );
    assert_eq!(refusal.output, "This folder is not a repository.");
    // A refused call never executes.
    assert_eq!(tool.executions().len(), 1);
}

#[tokio::test]
async fn a_tool_is_offered_where_its_marker_lies_and_a_section_learns_the_offered_tools() {
    let setup = Setup::new().await;
    let read = Arc::new(PathTool::new(READ_PATH, AccessMode::Read));
    let marked = Arc::new(PathTool::new(MARKED, AccessMode::Read));
    let marked_tool: Arc<dyn Tool> = Arc::<PathTool>::clone(&marked);
    let section = Arc::new(Recording::default());
    add_plugin(
        &setup.harness.runtime,
        PLUGIN,
        plugin(
            vec![
                reader(&read).offered_where(ToolOffering::InWorkspace),
                PluginTool::new(marked_tool, AgentSelector::Conversations)
                    .with_file_reads()
                    .offered_where(ToolOffering::WhereWorkspaceHolds(MARKER.to_owned())),
            ],
            Some(&section),
        ),
    )
    .unwrap();
    setup.harness.model.push_reply(reply("Without the marker."));
    setup.harness.model.push_reply(reply("With the marker."));

    let (first, first_run) = setup.start("OWNER: before the marker").await;
    setup.harness.wait_for_run(&first, &first_run).await;
    fs::create_dir(setup.root.join(MARKER)).unwrap();
    let (second, second_run) = setup.start("OWNER: after the marker").await;
    setup.harness.wait_for_run(&second, &second_run).await;

    let without = tool_names(&setup.harness.model.requests_for("OWNER: before the marker")[0]);
    let with = tool_names(&setup.harness.model.requests_for("OWNER: after the marker")[0]);
    assert!(without.contains(&READ_PATH.to_owned()), "{without:?}");
    assert!(!without.contains(&MARKED.to_owned()), "{without:?}");
    assert!(with.contains(&MARKED.to_owned()), "{with:?}");
    let offered = section.offered.lock().unwrap().clone();
    assert!(
        offered
            .iter()
            .any(|tools| tools.contains(&READ_PATH.to_owned())),
        "{offered:?}"
    );
    let request = &setup.harness.model.requests_for("OWNER: after the marker")[0];
    assert!(
        request
            .instructions
            .contains("Files may be read with files_read.")
    );
}
