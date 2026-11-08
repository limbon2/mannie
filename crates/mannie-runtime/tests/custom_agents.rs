//! Custom agents (docs/features/custom-agents.md; ADR-0059): a definition that
//! the owner wrote only narrows what its base has, a conversation chooses its
//! agent, an agent that only reads reaches nothing that acts on any path, and a
//! deleted agent never fails a run internally.

#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::host::ID_TAKEN;
use mannie_plugin_sdk::points::agents::{AgentBudgets, AgentSpec, Agents, Requester};
use mannie_plugin_sdk::points::executors::{Executors, TaskExecutor};
use mannie_plugin_sdk::points::sections::{
    Section, SectionBody, SectionContext, SectionSource, Sections,
};
use mannie_plugin_sdk::points::tools::{
    AgentSelector, PluginTool, Tool, ToolAccess, ToolCall, ToolExecution, ToolRefusal, ToolResult,
    ToolSpec, Tools,
};
use mannie_provider::testing::Step;
use mannie_provider::{Content, ModelRequest, ProviderEvent, Role};
use mannie_runtime::testing::{
    FAKE_EXECUTOR_KIND, FAKE_EXECUTOR_TOOL, FakeExecutor, Harness, HarnessOptions, add_plugin,
    executor_host, fast_config, reply, tool_calls,
};
use mannie_runtime::{ORCHESTRATOR_KEY, PLAN_KEY, RuntimeConfig, texts};
use mannie_types::agents::{AgentDraft, AgentError, AgentField, AgentKind, AgentStep, AgentView};
use mannie_types::chat::{ChatError, SendMessage, SetConversationAgent, StartConversation};
use mannie_types::ids::{ConversationId, ProfileId, RunId};
use mannie_types::provider::{CreateProviderProfile, ProfileAccess, ProfileDraft, ReasoningEffort};
use mannie_types::run::{RunFailure, RunState};
use mannie_types::tasks::{TaskState, UpdateChildAgentsSettings};
use mannie_types::tools::{SetFullAccess, UpdateToolsSettings};
use serde_json::{Value, json};

const READ_TOOLS: [&str; 7] = [
    "system_info",
    "list_directory",
    "file_info",
    "find_files",
    "search_text",
    "read_file",
    "read_tool_output",
];

/// A plugin's tool that acts, and one that reads, offered as Git's and SSH's
/// are: by the names of the built-in agents.
const ACTS: &str = "fake_acts";
const READS: &str = "fake_reads";
const PLUGIN: &str = "echo";

fn names(tools: &[&str]) -> Vec<String> {
    tools.iter().map(|tool| (*tool).to_owned()).collect()
}

fn with_tasks(tools: &[&str]) -> Vec<String> {
    let mut all = names(tools);
    all.extend(names(&["start_task", "follow_up_task", "wait_for_tasks"]));
    all
}

fn conversation_agent(name: &str, tools: Vec<String>) -> AgentDraft {
    AgentDraft {
        name: name.to_owned(),
        description: String::new(),
        kind: AgentKind::Conversation,
        read_only: false,
        profile_id: None,
        instructions: format!("{name} INSTRUCTIONS."),
        steps: Vec::new(),
        tools,
        plugins: None,
        children: None,
        max_tasks: 20,
        max_tool_rounds: 100,
        max_tool_calls: 250,
    }
}

fn read_only_agent(name: &str) -> AgentDraft {
    AgentDraft {
        read_only: true,
        ..conversation_agent(name, with_tasks(&READ_TOOLS))
    }
}

fn child_agent(name: &str, read_only: bool) -> AgentDraft {
    AgentDraft {
        kind: AgentKind::Child,
        description: format!("{name} DESCRIPTION."),
        read_only,
        max_tasks: 0,
        ..conversation_agent(name, names(&READ_TOOLS))
    }
}

struct Setup {
    harness: Harness,
}

impl Setup {
    async fn new() -> Self {
        Self::with_config(fast_config()).await
    }

    async fn with_config(config: RuntimeConfig) -> Self {
        let harness = Harness::with_options(HarnessOptions {
            use_tools: true,
            config,
            ..HarnessOptions::default()
        })
        .await;
        Self { harness }
    }

    async fn create(&self, draft: AgentDraft) -> AgentView {
        self.harness.runtime.create_agent(&draft).await.unwrap()
    }

    async fn start(&self, text: &str, agent: Option<&str>) -> (ConversationId, RunId) {
        let started = self
            .harness
            .runtime
            .start_conversation(StartConversation::new(text).with_agent(agent.map(str::to_owned)))
            .await
            .unwrap();
        (started.conversation_id, started.run_id)
    }

    async fn send(&self, conversation: &ConversationId, text: &str) -> RunId {
        self.harness
            .runtime
            .send_message(SendMessage::new(conversation.clone(), text.to_owned()))
            .await
            .unwrap()
            .run_id
    }

    async fn switch(&self, conversation: &ConversationId, agent: Option<&str>) {
        self.harness
            .runtime
            .set_conversation_agent(SetConversationAgent {
                conversation_id: conversation.clone(),
                agent_key: agent.map(str::to_owned),
            })
            .await
            .unwrap();
    }

    fn turn_on_fake_plugin(&self) {
        let tools = Contributions::new()
            .add::<Tools>(
                PluginTool::new(
                    Arc::new(Named(ACTS)),
                    AgentSelector::Agents(names(&["conversation", "general"])),
                )
                .with_side_effects(),
            )
            .add::<Tools>(
                PluginTool::new(
                    Arc::new(Named(READS)),
                    AgentSelector::Agents(names(&["conversation", "general", "explore"])),
                )
                .with_file_reads(),
            );
        add_plugin(&self.harness.runtime, PLUGIN, tools).unwrap();
    }

    fn turn_on_executor(&self) -> Arc<FakeExecutor> {
        let executor = FakeExecutor::new();
        let contributed: Arc<dyn TaskExecutor> = Arc::<FakeExecutor>::clone(&executor);
        add_plugin(
            &self.harness.runtime,
            FAKE_EXECUTOR_KIND,
            Contributions::new().add::<Executors>(contributed),
        )
        .unwrap();
        executor
    }
}

/// A tool of the fake plugin, which only has a name.
#[derive(Debug)]
struct Named(&'static str);

impl Tool for Named {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            self.0,
            "Does nothing.",
            json!({
                "additionalProperties": false,
                "properties": {},
                "required": [],
                "type": "object",
            }),
        )
    }

    fn prepare(&self, _call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        async { Ok(Vec::new()) }.boxed()
    }

    fn execute(&self, _execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        async { ToolResult::new("done") }.boxed()
    }
}

fn tool_names(request: &ModelRequest) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
}

fn start_task(title: &str, agent: &str, instruction: &str) -> String {
    json!({
        "activity": format!("Starting {title}"),
        "title": title,
        "agent": agent,
        "instruction": instruction,
        "background": false,
    })
    .to_string()
}

fn follow_up(task: u32, instruction: &str) -> String {
    json!({
        "activity": "Following up",
        "task": task,
        "instruction": instruction,
        "background": false,
    })
    .to_string()
}

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
                ..
            } if id == call_id => Some(output.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no result for {call_id}"))
}

/// The roster that `start_task` offers.
fn roster(request: &ModelRequest) -> Vec<String> {
    let tool = request
        .tools
        .iter()
        .find(|tool| tool.name == "start_task")
        .expect("the request offers start_task");
    tool.parameters["properties"]["agent"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap().to_owned())
        .collect()
}

fn description_of_start_task(request: &ModelRequest) -> String {
    request
        .tools
        .iter()
        .find(|tool| tool.name == "start_task")
        .unwrap()
        .description
        .clone()
}

fn argument_of_start_task(request: &ModelRequest) -> Value {
    request
        .tools
        .iter()
        .find(|tool| tool.name == "start_task")
        .unwrap()
        .parameters["properties"]["agent"]["description"]
        .clone()
}

#[tokio::test]
async fn the_built_in_agent_sends_what_it_sent_before() {
    let setup = Setup::new().await;
    setup.harness.model.push_reply(reply("Hello."));

    let (conversation, run) = setup.start("Hi", None).await;
    setup.harness.wait_for_run(&conversation, &run).await;

    let request = &setup.harness.model.reply_requests()[0];
    assert_eq!(description_of_start_task(request), texts::START_TASK);
    assert_eq!(
        argument_of_start_task(request),
        Value::from(texts::TASK_AGENT_ARGUMENT)
    );
    assert_eq!(roster(request), ["explore", "general"]);
    assert!(request.instructions.contains(texts::TASKS_GUIDANCE));
    assert!(!request.instructions.contains(texts::ROSTER_HEADING));
    let tools = tool_names(request);
    assert!(tools.contains(&"write_file".to_owned()), "{tools:?}");
    assert!(tools.contains(&"ask_owner".to_owned()), "{tools:?}");
    assert!(!tools.contains(&"save_plan".to_owned()), "{tools:?}");
    let summary = setup
        .harness
        .store
        .conversation_summary(&conversation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.agent_key, None);
}

#[tokio::test]
async fn a_custom_agent_gets_its_instructions_and_only_its_tools() {
    let setup = Setup::new().await;
    let agent = setup
        .create(conversation_agent("Reader", names(&["read_file"])))
        .await;
    assert_eq!(agent.key, "custom.reader");
    setup.harness.model.push_reply(reply("Hello."));

    let (conversation, run) = setup.start("Hi", Some(&agent.key)).await;
    setup.harness.wait_for_run(&conversation, &run).await;

    let request = &setup.harness.model.reply_requests()[0];
    assert!(request.instructions.contains("Reader INSTRUCTIONS."));
    assert!(request.instructions.starts_with(texts::BASE_INSTRUCTIONS));
    let tools = tool_names(request);
    assert!(tools.contains(&"read_file".to_owned()), "{tools:?}");
    for absent in ["write_file", "run_command", "ask_owner", "start_task"] {
        assert!(!tools.contains(&absent.to_owned()), "{absent}: {tools:?}");
    }
    let summary = setup
        .harness
        .store
        .conversation_summary(&conversation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.agent_key.as_deref(), Some("custom.reader"));
}

#[tokio::test]
async fn a_conversation_uses_its_agent_for_each_reply_and_retry() {
    let setup = Setup::new().await;
    let agent = setup
        .create(conversation_agent("Mine", names(&["read_file"])))
        .await;
    let model = &setup.harness.model;
    model.push_reply(reply("One."));
    model.push_reply(vec![
        Step::Event(ProviderEvent::TextDelta("Two".to_owned())),
        Step::Hang,
    ]);
    model.push_reply(reply("Three."));
    model.push_reply(reply("Four."));

    let (conversation, run) = setup.start("Start", None).await;
    setup.harness.wait_for_run(&conversation, &run).await;
    setup.switch(&conversation, Some(&agent.key)).await;
    let second = setup.send(&conversation, "Next").await;
    // The model took this reply, so a stop does not leave it for the retry.
    setup
        .harness
        .wait_until(&conversation, |transcript| {
            transcript.reply_text(1) == "Two"
        })
        .await;
    setup.harness.runtime.stop_reply(&conversation).await;
    setup.harness.wait_for_run(&conversation, &second).await;
    let retried = setup
        .harness
        .runtime
        .retry_reply(&conversation)
        .await
        .unwrap();
    setup
        .harness
        .wait_for_run(&conversation, &retried.run_id)
        .await;
    setup.switch(&conversation, None).await;
    let fourth = setup.send(&conversation, "Back").await;
    setup.harness.wait_for_run(&conversation, &fourth).await;

    let requests = setup.harness.model.reply_requests();
    assert!(tool_names(&requests[0]).contains(&"write_file".to_owned()));
    assert!(!tool_names(&requests[1]).contains(&"write_file".to_owned()));
    assert!(requests[1].instructions.contains("Mine INSTRUCTIONS."));
    assert!(requests[2].instructions.contains("Mine INSTRUCTIONS."));
    assert!(!requests[3].instructions.contains("Mine INSTRUCTIONS."));
    assert!(tool_names(&requests[3]).contains(&"write_file".to_owned()));
}

#[tokio::test]
async fn a_key_that_names_no_conversation_agent_is_refused() {
    let setup = Setup::new().await;
    let child = setup.create(child_agent("Helper", false)).await;
    setup.harness.model.push_reply(reply("Hello."));

    for key in ["custom.missing", child.key.as_str(), "explore"] {
        let refused = setup
            .harness
            .runtime
            .start_conversation(StartConversation::new("Hi").with_agent(Some(key.to_owned())))
            .await;
        assert_eq!(refused.err(), Some(ChatError::AgentNotFound), "{key}");
    }
    let (conversation, run) = setup.start("Hi", None).await;
    setup.harness.wait_for_run(&conversation, &run).await;
    let refused = setup
        .harness
        .runtime
        .set_conversation_agent(SetConversationAgent {
            conversation_id: conversation,
            agent_key: Some(child.key),
        })
        .await;
    assert_eq!(refused, Err(ChatError::AgentNotFound));
}

#[tokio::test]
async fn the_name_of_the_built_in_agent_is_stored_as_none() {
    let setup = Setup::new().await;
    setup.harness.model.push_reply(reply("Hello."));

    let (conversation, run) = setup.start("Hi", Some("conversation")).await;
    setup.harness.wait_for_run(&conversation, &run).await;

    let summary = setup
        .harness
        .store
        .conversation_summary(&conversation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.agent_key, None);
}

#[tokio::test]
async fn a_custom_child_joins_the_roster_with_its_description() {
    let setup = Setup::new().await;
    let child = setup.create(child_agent("Reviewer", false)).await;
    let model = &setup.harness.model;
    model.push_reply_for(
        "OWNER:",
        tool_calls(&[(
            "call_1",
            "start_task",
            &start_task("Review it", &child.key, "CHILD: review it"),
        )]),
    );
    model.push_reply_for("OWNER:", reply("It is fine."));
    model.push_reply_for("CHILD:", reply("Reviewed."));
    model.push_reply_for("CHILD:", reply("Reviewed and checked."));

    let (conversation, run) = setup.start("OWNER: go", None).await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Completed));
    let parent = setup.harness.model.requests_for("OWNER:");
    assert_eq!(
        roster(&parent[0]),
        ["explore", "general", "custom.reviewer"]
    );
    assert_eq!(
        description_of_start_task(&parent[0]),
        texts::START_TASK_ANY_ROSTER
    );
    let instructions = &parent[0].instructions;
    assert!(
        instructions.contains(texts::ROSTER_HEADING),
        "{instructions}"
    );
    assert!(
        instructions.contains("- custom.reviewer: Reviewer DESCRIPTION."),
        "{instructions}"
    );
    assert!(instructions.contains(&format!("- explore: {}", texts::EXPLORE_DESCRIPTION)));
    let child_requests = setup.harness.model.requests_for("CHILD:");
    assert!(
        child_requests[0]
            .instructions
            .starts_with(texts::CHILD_BASE_INSTRUCTIONS)
    );
    assert!(
        child_requests[0]
            .instructions
            .contains("Reviewer INSTRUCTIONS.")
    );
    let child_tools = tool_names(&child_requests[0]);
    assert!(
        child_tools.contains(&"read_file".to_owned()),
        "{child_tools:?}"
    );
    assert!(
        !child_tools.contains(&"write_file".to_owned()),
        "{child_tools:?}"
    );
    assert!(
        !child_tools.contains(&"start_task".to_owned()),
        "{child_tools:?}"
    );
    let [task] = transcript.tasks(0).try_into().unwrap();
    assert_eq!(task.agent, "custom.reviewer");
    assert_eq!(task.state, TaskState::Reported);
}

#[tokio::test]
async fn an_agent_gives_tasks_only_to_the_children_it_names() {
    let setup = Setup::new().await;
    let reader = setup.create(child_agent("Reader", true)).await;
    setup.create(child_agent("Writer", false)).await;
    let mut draft = conversation_agent("Lead", with_tasks(&READ_TOOLS));
    draft.children = Some(vec![reader.key.clone()]);
    let lead = setup.create(draft).await;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        "start_task",
        &start_task("Write it", "custom.writer", "Write it."),
    )]));
    model.push_reply(reply("Could not."));

    let (conversation, run) = setup.start("Go", Some(&lead.key)).await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let requests = setup.harness.model.reply_requests();
    assert_eq!(roster(&requests[0]), std::slice::from_ref(&reader.key));
    assert!(requests[0].instructions.contains("Reader DESCRIPTION."));
    assert!(!requests[0].instructions.contains("Writer DESCRIPTION."));
    assert!(
        result(&requests[1], "call_1").contains("there is no agent"),
        "{}",
        result(&requests[1], "call_1")
    );
    assert_eq!(transcript.tasks(0).len(), 0);
}

#[tokio::test]
async fn an_agent_that_only_reads_is_offered_nothing_that_acts() {
    for full_access in [false, true] {
        let setup = Setup::new().await;
        setup.turn_on_fake_plugin();
        let _executor = setup.turn_on_executor();
        setup
            .harness
            .runtime
            .update_tools_settings(UpdateToolsSettings {
                allow_changes_in_workspace: true,
                command_environment: Vec::new(),
                full_access_for_new_conversations: full_access,
                wake_on_command_end: false,
            })
            .await
            .unwrap();
        setup.create(child_agent("Writer", false)).await;
        let reader = setup.create(child_agent("Reader", true)).await;
        let plan = setup.create(read_only_agent("Planner")).await;
        setup.harness.model.push_reply(reply("Here is the plan."));

        let (conversation, run) = setup.start("Plan it", Some(&plan.key)).await;
        setup.harness.wait_for_run(&conversation, &run).await;

        let request = &setup.harness.model.reply_requests()[0];
        let tools = tool_names(request);
        for absent in [
            "write_file",
            "edit_file",
            "create_directory",
            "copy_path",
            "move_path",
            "delete_path",
            "run_command",
            "start_command",
            "read_command_output",
            "stop_command",
            "move_to_project",
            ACTS,
            FAKE_EXECUTOR_TOOL,
        ] {
            assert!(!tools.contains(&absent.to_owned()), "{absent}: {tools:?}");
        }
        assert!(tools.contains(&READS.to_owned()), "{tools:?}");
        assert!(tools.contains(&"read_file".to_owned()), "{tools:?}");
        assert_eq!(roster(request), ["explore".to_owned(), reader.key.clone()]);
        assert!(
            !request.instructions.contains("Writer DESCRIPTION."),
            "{}",
            request.instructions
        );
    }
}

#[tokio::test]
async fn an_agent_that_only_reads_cannot_resume_a_child_that_acts() {
    let setup = Setup::new().await;
    let plan = setup.create(read_only_agent("Planner")).await;
    let model = &setup.harness.model;
    model.push_reply_for(
        "OWNER:",
        tool_calls(&[(
            "call_1",
            "start_task",
            &start_task("Do it", "general", "CHILD: do it"),
        )]),
    );
    model.push_reply_for("OWNER:", reply("Done."));
    model.push_reply_for("CHILD:", reply("Did it."));
    model.push_reply_for("CHILD:", reply("Did it, and checked."));
    model.push_reply_for(
        "OWNER:",
        tool_calls(&[(
            "call_2",
            "follow_up_task",
            &follow_up(1, "CHILD: change it again"),
        )]),
    );
    model.push_reply_for("OWNER:", reply("I cannot."));

    let (conversation, run) = setup.start("OWNER: first", None).await;
    setup.harness.wait_for_run(&conversation, &run).await;
    let child_requests = setup.harness.model.requests_for("CHILD:").len();
    setup.switch(&conversation, Some(&plan.key)).await;
    let second = setup.send(&conversation, "OWNER: second").await;
    let transcript = setup.harness.wait_for_run(&conversation, &second).await;

    assert_eq!(transcript.run_state(&second), Some(RunState::Completed));
    let parent = setup.harness.model.requests_for("OWNER:");
    assert_eq!(
        result(parent.last().unwrap(), "call_2"),
        texts::TASK_NOT_OFFERED
    );
    assert_eq!(
        setup.harness.model.requests_for("CHILD:").len(),
        child_requests,
        "the child that acts got no further task"
    );
}

#[tokio::test]
async fn an_agent_that_only_reads_cannot_continue_a_workers_task() {
    let setup = Setup::new().await;
    let executor = setup.turn_on_executor();
    let plan = setup.create(read_only_agent("Planner")).await;
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        FAKE_EXECUTOR_TOOL,
        &json!({
            "activity": "Giving it",
            "target": "box-01",
            "title": "Build it",
            "instruction": "Build it.",
            "background": false,
        })
        .to_string(),
    )]));
    model.push_reply(reply("Built."));
    model.push_reply(tool_calls(&[(
        "call_2",
        "follow_up_task",
        &follow_up(1, "Now change the host."),
    )]));
    model.push_reply(reply("I cannot."));

    let (conversation, run) = setup.start("Build it", None).await;
    let submission = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(first) = executor.submissions().into_iter().next() {
                return first;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    executor_host(&setup.harness.runtime, FAKE_EXECUTOR_KIND)
        .publish_task_state(FakeExecutor::reported(&submission, 2, "Built."))
        .await
        .unwrap();
    setup.harness.wait_for_run(&conversation, &run).await;

    setup.switch(&conversation, Some(&plan.key)).await;
    let second = setup.send(&conversation, "Again").await;
    setup.harness.wait_for_run(&conversation, &second).await;

    let last = setup.harness.model.reply_requests().pop().unwrap();
    assert_eq!(result(&last, "call_2"), texts::TASK_NOT_OFFERED);
    assert_eq!(executor.submissions().len(), 1);
}

#[tokio::test]
async fn a_read_only_child_that_was_edited_to_act_leaves_the_roster() {
    let setup = Setup::new().await;
    let reader = setup.create(child_agent("Reader", true)).await;
    let plan = setup.create(read_only_agent("Planner")).await;
    setup
        .harness
        .runtime
        .update_agent(&reader.key, &child_agent("Reader", false))
        .await
        .unwrap();
    setup.harness.model.push_reply(reply("Planned."));

    let (conversation, run) = setup.start("Plan", Some(&plan.key)).await;
    setup.harness.wait_for_run(&conversation, &run).await;

    let request = &setup.harness.model.reply_requests()[0];
    assert_eq!(roster(request), ["explore"]);
}

#[tokio::test]
async fn a_definition_gets_only_the_plugins_it_names() {
    let setup = Setup::new().await;
    setup.turn_on_fake_plugin();
    let mut none = conversation_agent("None", names(&["read_file"]));
    none.plugins = Some(Vec::new());
    let none = setup.create(none).await;
    let mut named = conversation_agent("Named", names(&["read_file"]));
    named.plugins = Some(vec![PLUGIN.to_owned()]);
    let named = setup.create(named).await;
    let every = setup
        .create(conversation_agent("Every", names(&["read_file"])))
        .await;
    let model = &setup.harness.model;
    model.push_reply(reply("One."));
    model.push_reply(reply("Two."));
    model.push_reply(reply("Three."));

    for agent in [&none, &named, &every] {
        let (conversation, run) = setup.start("Hi", Some(&agent.key)).await;
        setup.harness.wait_for_run(&conversation, &run).await;
    }

    let requests = setup.harness.model.reply_requests();
    assert!(!tool_names(&requests[0]).contains(&ACTS.to_owned()));
    assert!(tool_names(&requests[1]).contains(&ACTS.to_owned()));
    assert!(tool_names(&requests[2]).contains(&ACTS.to_owned()));
}

/// A plugin's section that says which agent it was rendered for.
#[derive(Debug)]
struct AgentEcho;

impl SectionSource for AgentEcho {
    fn render(&self, context: &SectionContext) -> Option<String> {
        Some(format!("SECTION FOR {}.", context.agent))
    }
}

#[tokio::test]
async fn a_plugin_sees_a_custom_agent_as_its_base_and_only_when_it_gets_the_plugin() {
    let setup = Setup::new().await;
    add_plugin(
        &setup.harness.runtime,
        PLUGIN,
        Contributions::new().add::<Sections>(Section::new(
            "fake",
            400,
            SectionBody::Source(Arc::new(AgentEcho)),
        )),
    )
    .unwrap();
    let acts = setup
        .create(conversation_agent("Acts", names(&["read_file"])))
        .await;
    let reads = setup.create(read_only_agent("Reads")).await;
    let mut without = conversation_agent("Without", names(&["read_file"]));
    without.plugins = Some(Vec::new());
    let without = setup.create(without).await;
    let model = &setup.harness.model;
    model.push_reply(reply("One."));
    model.push_reply(reply("Two."));
    model.push_reply(reply("Three."));

    for agent in [&acts, &reads, &without] {
        let (conversation, run) = setup.start("Hi", Some(&agent.key)).await;
        setup.harness.wait_for_run(&conversation, &run).await;
    }

    let requests = setup.harness.model.reply_requests();
    assert!(
        requests[0]
            .instructions
            .contains("SECTION FOR conversation.")
    );
    assert!(requests[1].instructions.contains("SECTION FOR explore."));
    assert!(!requests[2].instructions.contains("SECTION FOR"));
}

#[tokio::test]
async fn a_plugin_cannot_take_the_prefix_of_the_owners_agents() {
    let setup = Setup::new().await;

    let refused = add_plugin(
        &setup.harness.runtime,
        PLUGIN,
        Contributions::new().add::<Agents>(AgentSpec::new(
            "custom.plan",
            Requester::Owner,
            "Take over.",
            &["write_file"],
            AgentBudgets::new(1, 1, 1, 0),
        )),
    )
    .unwrap_err();

    assert_eq!(refused.code, ID_TAKEN);
}

#[tokio::test]
async fn deleting_an_agent_sets_its_conversations_back_and_a_new_reply_uses_the_built_in_one() {
    let setup = Setup::new().await;
    let agent = setup
        .create(conversation_agent("Mine", names(&["read_file"])))
        .await;
    let model = &setup.harness.model;
    model.push_reply(reply("One."));
    model.push_reply(reply("Two."));

    let (conversation, run) = setup.start("Hi", Some(&agent.key)).await;
    setup.harness.wait_for_run(&conversation, &run).await;
    setup
        .harness
        .runtime
        .delete_agent(&agent.key)
        .await
        .unwrap();

    let summary = setup
        .harness
        .store
        .conversation_summary(&conversation)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.agent_key, None);
    let second = setup.send(&conversation, "Again").await;
    let transcript = setup.harness.wait_for_run(&conversation, &second).await;
    assert_eq!(transcript.run_state(&second), Some(RunState::Completed));
    let requests = setup.harness.model.reply_requests();
    assert!(tool_names(&requests[1]).contains(&"write_file".to_owned()));
    assert_eq!(
        setup.harness.runtime.delete_agent(&agent.key).await,
        Err(AgentError::NotFound)
    );
}

#[tokio::test]
async fn deleting_an_agent_leaves_a_reply_that_runs_with_what_it_started() {
    let setup = Setup::new().await;
    let agent = setup
        .create(conversation_agent("Mine", names(&["read_file"])))
        .await;
    let model = &setup.harness.model;
    model.push_reply(vec![Step::Hang]);

    let (conversation, run) = setup.start("Hi", Some(&agent.key)).await;
    setup
        .harness
        .wait_until(&conversation, |transcript| transcript.run(&run).is_some())
        .await;
    setup
        .harness
        .runtime
        .delete_agent(&agent.key)
        .await
        .unwrap();
    setup.harness.runtime.stop_reply(&conversation).await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    assert_eq!(transcript.run_state(&run), Some(RunState::Stopped));
}

#[tokio::test]
async fn a_task_for_a_deleted_child_is_refused() {
    let setup = Setup::new().await;
    let child = setup.create(child_agent("Helper", false)).await;
    let model = &setup.harness.model;
    model.push_reply_for(
        "OWNER:",
        tool_calls(&[(
            "call_1",
            "start_task",
            &start_task("Help", &child.key, "CHILD: help"),
        )]),
    );
    model.push_reply_for("OWNER:", reply("Helped."));
    model.push_reply_for("CHILD:", reply("Helped."));
    model.push_reply_for("CHILD:", reply("Helped, and checked."));
    model.push_reply_for(
        "OWNER:",
        tool_calls(&[("call_2", "follow_up_task", &follow_up(1, "CHILD: more"))]),
    );
    model.push_reply_for("OWNER:", reply("It is gone."));

    let (conversation, run) = setup.start("OWNER: first", None).await;
    setup.harness.wait_for_run(&conversation, &run).await;
    setup
        .harness
        .runtime
        .delete_agent(&child.key)
        .await
        .unwrap();
    let second = setup.send(&conversation, "OWNER: second").await;
    let transcript = setup.harness.wait_for_run(&conversation, &second).await;

    assert_eq!(transcript.run_state(&second), Some(RunState::Completed));
    let parent = setup.harness.model.requests_for("OWNER:");
    assert_eq!(
        result(parent.last().unwrap(), "call_2"),
        texts::TASK_AGENT_GONE
    );
}

#[tokio::test]
async fn a_child_whose_agent_was_deleted_in_the_queue_fails_and_says_why() {
    let setup = Setup::new().await;
    setup
        .harness
        .runtime
        .update_child_agents_settings(UpdateChildAgentsSettings {
            parallel_children: 1,
        })
        .await
        .unwrap();
    let child = setup.create(child_agent("Helper", false)).await;
    let model = &setup.harness.model;
    model.push_reply_for(
        "OWNER:",
        tool_calls(&[
            (
                "call_1",
                "start_task",
                &start_task("Hold", "general", "CHILD-A: hold the slot"),
            ),
            (
                "call_2",
                "start_task",
                &start_task("Help", &child.key, "CHILD-B: help"),
            ),
        ]),
    );
    model.push_reply_for("OWNER:", reply("One of them failed."));
    model.push_reply_for("CHILD-A:", vec![Step::Hang]);

    let (conversation, run) = setup.start("OWNER: go", None).await;
    setup
        .harness
        .wait_until(&conversation, |transcript| {
            let tasks = transcript.tasks(0);
            tasks.len() == 2
                && tasks.iter().any(|task| task.state == TaskState::Running)
                && tasks.iter().any(|task| task.state == TaskState::Accepted)
        })
        .await;
    setup
        .harness
        .runtime
        .delete_agent(&child.key)
        .await
        .unwrap();
    let holding = setup.harness.snapshot(&conversation).await.tasks(0)[0].clone();
    setup
        .harness
        .runtime
        .stop_reply(&holding.conversation_id)
        .await;
    let transcript = setup.harness.wait_for_run(&conversation, &run).await;

    let tasks = transcript.tasks(0);
    let helper = tasks
        .iter()
        .find(|task| task.agent == child.key)
        .expect("the task of the deleted agent");
    assert_eq!(helper.state, TaskState::Failed);
    assert_eq!(helper.failure, Some(RunFailure::AgentMissing));
}

async fn second_profile(setup: &Setup, model: &str, use_tools: bool) -> ProfileId {
    setup
        .harness
        .runtime
        .create_profile(CreateProviderProfile {
            draft: ProfileDraft {
                access: ProfileAccess::Endpoint,
                name: model.to_owned(),
                interface: setup.harness.profile.interface,
                base_url: "https://api.example.com/v1".to_owned(),
                model: model.to_owned(),
                context_window: 200_000,
                output_limit: 16_000,
                reasoning_effort: ReasoningEffort::default(),
                use_tools,
            },
            credential: None,
        })
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn a_custom_child_uses_its_own_profile_and_falls_back_to_the_parents() {
    let setup = Setup::new().await;
    let own = second_profile(&setup, "own-model", true).await;
    let no_tools = second_profile(&setup, "no-tools-model", false).await;
    let parent_model = setup.harness.profile.model.clone();
    let mut with_own = child_agent("WithOwn", false);
    with_own.profile_id = Some(own.clone());
    let with_own = setup.create(with_own).await;
    let mut without_tools = child_agent("WithoutTools", false);
    without_tools.profile_id = Some(no_tools);
    let without_tools = setup.create(without_tools).await;
    let model = &setup.harness.model;
    model.push_reply_for(
        "OWNER:",
        tool_calls(&[
            (
                "call_1",
                "start_task",
                &start_task("A", &with_own.key, "CHILD-A: own"),
            ),
            (
                "call_2",
                "start_task",
                &start_task("B", &without_tools.key, "CHILD-B: no tools"),
            ),
            (
                "call_3",
                "start_task",
                &start_task("C", "general", "CHILD-C: built in"),
            ),
        ]),
    );
    model.push_reply_for("OWNER:", reply("Done."));
    for marker in ["CHILD-A:", "CHILD-B:", "CHILD-C:"] {
        model.push_reply_for(marker, reply("Did it."));
        model.push_reply_for(marker, reply("Did it, and checked."));
    }

    let (conversation, run) = setup.start("OWNER: go", None).await;
    setup.harness.wait_for_run(&conversation, &run).await;

    let requests = &setup.harness.model;
    assert_eq!(requests.requests_for("CHILD-A:")[0].model, "own-model");
    assert_eq!(requests.requests_for("CHILD-B:")[0].model, parent_model);
    assert_eq!(requests.requests_for("CHILD-C:")[0].model, parent_model);
}

#[tokio::test]
async fn choosing_an_agent_with_a_profile_sets_the_conversations_profile() {
    let setup = Setup::new().await;
    let own = second_profile(&setup, "own-model", true).await;
    let mut draft = conversation_agent("Mine", names(&["read_file"]));
    draft.profile_id = Some(own.clone());
    let agent = setup.create(draft).await;
    let model = &setup.harness.model;
    model.push_reply(reply("One."));
    model.push_reply(reply("Two."));

    let (started, run) = setup.start("Hi", Some(&agent.key)).await;
    setup.harness.wait_for_run(&started, &run).await;
    let (other, run) = setup.start("Hi", None).await;
    setup.harness.wait_for_run(&other, &run).await;
    setup.switch(&other, Some(&agent.key)).await;

    for conversation in [&started, &other] {
        let summary = setup
            .harness
            .store
            .conversation_summary(conversation)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(summary.profile_id, Some(own.clone()));
    }
    assert_eq!(setup.harness.model.reply_requests()[0].model, "own-model");
}

#[tokio::test]
async fn steps_reach_the_model_as_stages_and_a_deleted_child_as_work_for_the_agent() {
    let setup = Setup::new().await;
    let child = setup.create(child_agent("Helper", false)).await;
    let mut draft = conversation_agent("Lead", with_tasks(&READ_TOOLS));
    draft.tools.push("ask_owner".to_owned());
    draft.steps = vec![
        AgentStep {
            title: "Look".to_owned(),
            instruction: "Find it.".to_owned(),
            agent: Some("explore".to_owned()),
            several: true,
            ask_first: false,
        },
        AgentStep {
            title: "Help".to_owned(),
            instruction: "Do it.".to_owned(),
            agent: Some(child.key.clone()),
            several: false,
            ask_first: true,
        },
        AgentStep {
            title: "Sum up".to_owned(),
            instruction: "Say what happened.".to_owned(),
            agent: None,
            several: false,
            ask_first: false,
        },
    ];
    let lead = setup.create(draft).await;
    let model = &setup.harness.model;
    model.push_reply(reply("One."));
    model.push_reply(reply("Two."));

    let (conversation, run) = setup.start("Go", Some(&lead.key)).await;
    setup.harness.wait_for_run(&conversation, &run).await;
    setup
        .harness
        .runtime
        .delete_agent(&child.key)
        .await
        .unwrap();
    let (conversation, run) = setup.start("Go", Some(&lead.key)).await;
    setup.harness.wait_for_run(&conversation, &run).await;

    let requests = setup.harness.model.reply_requests();
    let first = &requests[0].instructions;
    assert!(first.contains(texts::STAGES_HEADING), "{first}");
    assert!(
        first.contains("1. Look: Find it. Give this to explore agents, several at the same time."),
        "{first}"
    );
    assert!(
        first.contains(
            "2. Help: Do it. Give this to the custom.helper agent. Ask the owner with ask_owner before you start this stage."
        ),
        "{first}"
    );
    assert!(
        first.contains("3. Sum up: Say what happened. Do this yourself."),
        "{first}"
    );
    let second = &requests[1].instructions;
    assert!(
        second.contains("2. Help: Do it. Do this yourself. Ask the owner"),
        "{second}"
    );
}

#[tokio::test]
async fn the_guidance_on_tasks_does_not_offer_commands_that_the_agent_lacks() {
    let setup = Setup::new().await;
    let lead = setup
        .create(conversation_agent("Lead", with_tasks(&READ_TOOLS)))
        .await;
    setup.harness.model.push_reply(reply("Hello."));

    let (conversation, run) = setup.start("Hi", Some(&lead.key)).await;
    setup.harness.wait_for_run(&conversation, &run).await;

    let request = &setup.harness.model.reply_requests()[0];
    assert!(
        request
            .instructions
            .contains(texts::TASKS_GUIDANCE_WITHOUT_COMMANDS),
        "{}",
        request.instructions
    );
    assert!(!request.instructions.contains(texts::TASKS_GUIDANCE));
}

#[tokio::test]
async fn an_agent_that_cannot_start_commands_is_not_woken_and_reads_no_missing_tool() {
    let setup = Setup::with_config(RuntimeConfig {
        wake_up_delay: Duration::from_millis(50),
        ..fast_config()
    })
    .await;
    setup
        .harness
        .runtime
        .update_tools_settings(UpdateToolsSettings {
            allow_changes_in_workspace: true,
            command_environment: Vec::new(),
            full_access_for_new_conversations: true,
            wake_on_command_end: true,
        })
        .await
        .unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let project = setup
        .harness
        .runtime
        .add_project(workspace.path().to_str().unwrap().to_owned())
        .await
        .unwrap();
    let lead = setup
        .create(conversation_agent("Lead", with_tasks(&READ_TOOLS)))
        .await;
    let quick = if cfg!(windows) {
        "Start-Sleep -Seconds 2; exit 0"
    } else {
        "sleep 2; exit 0"
    };
    let model = &setup.harness.model;
    model.push_reply(tool_calls(&[(
        "call_1",
        "start_command",
        &json!({ "command": quick }).to_string(),
    )]));
    model.push_reply(reply("It runs."));
    model.push_reply(reply("I see it ended."));

    let started = setup
        .harness
        .runtime
        .start_conversation(StartConversation::new("Run it").with_project(Some(project.id)))
        .await
        .unwrap();
    let conversation = started.conversation_id;
    setup
        .harness
        .wait_for_run(&conversation, &started.run_id)
        .await;
    setup.switch(&conversation, Some(&lead.key)).await;
    tokio::time::sleep(Duration::from_secs(4)).await;

    let before = setup.harness.model.reply_requests().len();
    assert_eq!(before, 2, "no end woke the agent that cannot read it");
    let second = setup.send(&conversation, "Next").await;
    setup.harness.wait_for_run(&conversation, &second).await;
    let last = setup.harness.model.reply_requests().pop().unwrap();
    let said: String = last
        .messages
        .iter()
        .filter(|message| message.role == Role::User)
        .flat_map(|message| &message.content)
        .filter_map(|content| match content {
            Content::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        said.contains("Background commands of this conversation"),
        "{said}"
    );
    assert!(!said.contains("read_command_output"), "{said}");
    setup.harness.runtime.shutdown().await;
}

#[tokio::test]
async fn the_kinds_list_what_each_may_name_and_a_forged_draft_is_refused() {
    let setup = Setup::new().await;
    let runtime = &setup.harness.runtime;
    let refused_field = |error: Result<AgentView, AgentError>| match error {
        Err(AgentError::FieldNotValid { field }) => Some(field),
        _ => None,
    };

    let mut child_asks = child_agent("A", false);
    child_asks.tools.push("ask_owner".to_owned());
    assert_eq!(
        refused_field(runtime.create_agent(&child_asks).await),
        Some(AgentField::Tools)
    );
    let mut child_moves = child_agent("B", false);
    child_moves.tools.push("move_to_project".to_owned());
    assert_eq!(
        refused_field(runtime.create_agent(&child_moves).await),
        Some(AgentField::Tools)
    );
    let mut reads_and_writes = read_only_agent("C");
    reads_and_writes.tools.push("write_file".to_owned());
    assert_eq!(
        refused_field(runtime.create_agent(&reads_and_writes).await),
        Some(AgentField::Tools)
    );
    let mut made_up = conversation_agent("D", names(&["made_up"]));
    made_up.max_tasks = 0;
    assert_eq!(
        refused_field(runtime.create_agent(&made_up).await),
        Some(AgentField::Tools)
    );
    let mut unknown_child = conversation_agent("E", with_tasks(&READ_TOOLS));
    unknown_child.children = Some(names(&["custom.nobody"]));
    assert_eq!(
        refused_field(runtime.create_agent(&unknown_child).await),
        Some(AgentField::Children)
    );
    let mut reads_general = read_only_agent("F");
    reads_general.children = Some(names(&["general"]));
    assert_eq!(
        refused_field(runtime.create_agent(&reads_general).await),
        Some(AgentField::Children)
    );
    let mut step_without_tasks = conversation_agent("G", names(&["read_file"]));
    step_without_tasks.steps = vec![AgentStep {
        title: "Look".to_owned(),
        instruction: "Look.".to_owned(),
        agent: Some("explore".to_owned()),
        several: false,
        ask_first: false,
    }];
    assert_eq!(
        refused_field(runtime.create_agent(&step_without_tasks).await),
        Some(AgentField::Steps)
    );
    let mut missing_profile = conversation_agent("H", names(&["read_file"]));
    missing_profile.profile_id =
        Some(ProfileId::parse("01890000-0000-7000-8000-000000000000").unwrap());
    assert_eq!(
        refused_field(runtime.create_agent(&missing_profile).await),
        Some(AgentField::Profile)
    );
}

#[tokio::test]
async fn names_stay_unique_and_a_kind_and_a_built_in_agent_stay_as_they_are() {
    let setup = Setup::new().await;
    let runtime = &setup.harness.runtime;
    let agent = setup
        .create(conversation_agent("Mine", names(&["read_file"])))
        .await;

    let same = runtime
        .create_agent(&conversation_agent("MINE", names(&["read_file"])))
        .await;
    assert_eq!(same.err(), Some(AgentError::NameTaken));
    let builtin_name = runtime
        .create_agent(&conversation_agent("Explore", names(&["read_file"])))
        .await;
    assert_eq!(builtin_name.err(), Some(AgentError::NameTaken));
    let kind = runtime
        .update_agent(&agent.key, &child_agent("Mine", false))
        .await;
    assert_eq!(kind.err(), Some(AgentError::KindChanged));
    let builtin = runtime
        .update_agent("explore", &child_agent("Explore 2", true))
        .await;
    assert_eq!(builtin.err(), Some(AgentError::Builtin));
    assert_eq!(
        runtime.delete_agent("explore").await,
        Err(AgentError::Builtin)
    );
    assert_eq!(
        runtime.delete_agent("nothing").await,
        Err(AgentError::NotFound)
    );

    let renamed = runtime
        .update_agent(
            &agent.key,
            &conversation_agent("Renamed", names(&["read_file"])),
        )
        .await
        .unwrap();
    assert_eq!(renamed.key, agent.key);
    assert_eq!(renamed.draft.name, "Renamed");
    let keys: Vec<String> = runtime
        .list_agents()
        .into_iter()
        .map(|view| view.key)
        .collect();
    assert_eq!(keys, ["conversation", "explore", "general", "custom.mine"]);
}

#[tokio::test]
async fn the_starters_are_written_once_and_survive_a_restart_as_plain_rows() {
    let setup = Setup::new().await;
    let runtime = &setup.harness.runtime;

    runtime.write_starter_agents().await.unwrap();
    let keys = |views: Vec<AgentView>| -> Vec<String> {
        views
            .into_iter()
            .filter(|view| !view.builtin)
            .map(|view| view.key)
            .collect()
    };
    assert_eq!(keys(runtime.list_agents()), [ORCHESTRATOR_KEY, PLAN_KEY]);

    runtime.delete_agent(PLAN_KEY).await.unwrap();
    runtime.write_starter_agents().await.unwrap();
    assert_eq!(keys(runtime.list_agents()), [ORCHESTRATOR_KEY]);

    let restarted = setup.harness.restart().await;
    assert_eq!(keys(restarted.runtime.list_agents()), [ORCHESTRATOR_KEY]);
    restarted.runtime.write_starter_agents().await.unwrap();
    assert_eq!(keys(restarted.runtime.list_agents()), [ORCHESTRATOR_KEY]);
}

#[tokio::test]
async fn the_orchestrator_has_no_tool_that_acts_and_still_gives_tasks_to_workers() {
    let setup = Setup::new().await;
    setup.turn_on_fake_plugin();
    let _executor = setup.turn_on_executor();
    setup.harness.runtime.write_starter_agents().await.unwrap();
    setup.harness.model.push_reply(reply("Hello."));

    let (conversation, run) = setup.start("Hi", Some(ORCHESTRATOR_KEY)).await;
    setup.harness.wait_for_run(&conversation, &run).await;

    let request = &setup.harness.model.reply_requests()[0];
    let tools = tool_names(request);
    for absent in ["write_file", "run_command", "start_command", ACTS, READS] {
        assert!(!tools.contains(&absent.to_owned()), "{absent}: {tools:?}");
    }
    for present in ["read_file", "start_task", "ask_owner", FAKE_EXECUTOR_TOOL] {
        assert!(tools.contains(&present.to_owned()), "{present}: {tools:?}");
    }
    assert!(
        request
            .instructions
            .contains(texts::ORCHESTRATOR_INSTRUCTIONS)
    );
    assert!(request.instructions.contains(texts::STAGES_HEADING));
}

#[tokio::test]
async fn full_access_changes_nothing_of_what_a_read_only_conversation_is_offered() {
    let setup = Setup::new().await;
    let plan = setup.create(read_only_agent("Planner")).await;
    let model = &setup.harness.model;
    model.push_reply(reply("One."));
    model.push_reply(reply("Two."));

    let (conversation, run) = setup.start("Hi", Some(&plan.key)).await;
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
    let second = setup.send(&conversation, "Again").await;
    setup.harness.wait_for_run(&conversation, &second).await;

    let requests = setup.harness.model.reply_requests();
    assert_eq!(tool_names(&requests[0]), tool_names(&requests[1]));
}
