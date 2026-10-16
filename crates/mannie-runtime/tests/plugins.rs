//! What plugins contribute to the runtime while they are on (ADR-0030,
//! decisions 5 and 8), through a fake plugin: its tool, its child agent and
//! its section reach the runs that start while it is on, a run keeps what it
//! started with, its calls into the plugin after the plugin stopped do not
//! run, and the runtime takes all of a plugin's contributions or none.

#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_provider::testing::Step;
use mannie_provider::{Content, ModelRequest, Role};
use mannie_runtime::testing::{FakeExecutor, Harness, HarnessOptions, reply, tool_calls};
use mannie_runtime::{
    AgentDefinition, ContributionError, GENERAL, InstructionSection, PluginContributions,
    PluginTool, texts,
};
use mannie_tools::{Done, Execution, PermissionClass, PrepareError, Prepared, Tool, ToolContext};
use mannie_types::chat::StartConversation;
use mannie_types::ids::{ConversationId, RunId};
use mannie_types::tools::{NotRunReason, ToolOutcome};
use serde_json::{Value, json};

/// The fake plugin's ID.
const PLUGIN: &str = "echo";

/// The fake plugin's tool.
const ECHO: &str = "echo_text";

/// The fake plugin's section.
const SECTION: &str = "The echo plugin repeats text with echo_text.";

/// The fake plugin's child agent.
const HELPER: &str = "helper";

/// A tool that answers with its argument.
#[derive(Debug)]
struct EchoTool {
    parameters: Value,
}

impl EchoTool {
    fn new() -> Self {
        Self {
            parameters: json!({
                "additionalProperties": false,
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
                "type": "object",
            }),
        }
    }
}

impl Tool for EchoTool {
    fn name(&self) -> &'static str {
        ECHO
    }

    fn description(&self) -> &'static str {
        "Repeats a text."
    }

    fn parameters(&self) -> &Value {
        &self.parameters
    }

    fn class(&self) -> PermissionClass {
        PermissionClass::ReadOnly
    }

    fn prepare(
        &self,
        _context: ToolContext,
        _arguments: String,
    ) -> BoxFuture<'static, Result<Prepared, PrepareError>> {
        async {
            Ok(Prepared {
                accesses: Vec::new(),
            })
        }
        .boxed()
    }

    fn execute(&self, execution: Execution) -> BoxFuture<'static, Done> {
        async move {
            Done {
                outcome: ToolOutcome::SystemInfo {
                    system: "echo".to_owned(),
                    shell: "echo".to_owned(),
                },
                output: format!("Echo: {}", execution.arguments),
                archives: Vec::new(),
            }
        }
        .boxed()
    }
}

/// The fake plugin's child agent.
const HELPER_AGENT: AgentDefinition = AgentDefinition {
    name: HELPER,
    ..GENERAL
};

fn contributions() -> PluginContributions {
    PluginContributions {
        tools: vec![PluginTool {
            tool: Arc::new(EchoTool::new()),
            agents: vec!["conversation"],
        }],
        agents: vec![HELPER_AGENT],
        sections: vec![InstructionSection::new("echo", 900, |_context| {
            Some(SECTION.to_owned())
        })],
        executors: Vec::new(),
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
        .start_conversation(StartConversation {
            text: text.to_owned(),
            profile_id: None,
            project_id: None,
        })
        .await
        .unwrap();
    (started.conversation_id, started.run_id)
}

fn tool_names(request: &ModelRequest) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
}

/// The child agents that `start_task` offers in a request.
fn child_agents(request: &ModelRequest) -> Vec<String> {
    request
        .tools
        .iter()
        .find(|tool| tool.name == "start_task")
        .and_then(|tool| {
            tool.parameters["properties"]["agent"]["enum"]
                .as_array()
                .cloned()
        })
        .unwrap_or_default()
        .iter()
        .filter_map(|agent| agent.as_str().map(str::to_owned))
        .collect()
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

/// Waits until the model got its first request of a conversation.
async fn until_requested(harness: &Harness, marker: &str) {
    let model = harness.model.clone();
    let marker = marker.to_owned();
    tokio::time::timeout(Duration::from_secs(20), async move {
        while model.requests_for(&marker).is_empty() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the model is called");
}

#[tokio::test]
async fn a_plugins_tool_agent_and_section_reach_the_runs_that_start_while_it_is_on() {
    let harness = harness().await;
    harness.runtime.add_plugin(PLUGIN, contributions()).unwrap();
    harness.model.push_reply(reply("On."));
    let (conversation, run) = start(&harness, "FIRST: hello").await;
    harness.wait_for_run(&conversation, &run).await;

    assert!(harness.runtime.remove_plugin(PLUGIN));
    harness.model.push_reply(reply("Off."));
    let (conversation, run) = start(&harness, "SECOND: hello").await;
    harness.wait_for_run(&conversation, &run).await;

    let on = &harness.model.requests_for("FIRST:")[0];
    let off = &harness.model.requests_for("SECOND:")[0];
    assert!(
        tool_names(on).contains(&ECHO.to_owned()),
        "{:?}",
        tool_names(on)
    );
    assert!(
        child_agents(on).contains(&HELPER.to_owned()),
        "{:?}",
        child_agents(on)
    );
    assert!(on.instructions.contains(SECTION));
    assert!(!tool_names(off).contains(&ECHO.to_owned()));
    assert!(!child_agents(off).contains(&HELPER.to_owned()));
    assert!(!off.instructions.contains(SECTION));
    assert!(!harness.runtime.remove_plugin(PLUGIN), "it was off already");
}

#[tokio::test]
async fn a_run_keeps_what_it_started_with_and_its_calls_into_a_stopped_plugin_do_not_run() {
    let harness = harness().await;
    harness.runtime.add_plugin(PLUGIN, contributions()).unwrap();
    let mut held = vec![Step::Delay(Duration::from_millis(300))];
    held.extend(tool_calls(&[
        (
            "call_1",
            ECHO,
            &json!({ "activity": "Echoing", "text": "hi" }).to_string(),
        ),
        (
            "call_2",
            "start_task",
            &json!({
                "activity": "Starting",
                "title": "Help",
                "agent": HELPER,
                "instruction": "Help with it.",
                "background": false,
            })
            .to_string(),
        ),
    ]));
    harness.model.push_reply(held);
    harness.model.push_reply(reply("The plugin is off."));

    let (conversation, run) = start(&harness, "OWNER: echo it").await;
    until_requested(&harness, "OWNER:").await;
    harness.runtime.remove_plugin(PLUGIN);
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let requests = harness.model.requests_for("OWNER:");
    assert_eq!(
        result(&requests[1], "call_1"),
        texts::tool_plugin_off(PLUGIN)
    );
    assert_eq!(
        result(&requests[1], "call_2"),
        texts::tool_plugin_off(PLUGIN)
    );
    // The prompt and the tools stay as they were when the reply started.
    assert!(requests[1].instructions.contains(SECTION));
    assert!(tool_names(&requests[1]).contains(&ECHO.to_owned()));
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(
        call.not_run,
        Some(NotRunReason::PluginOff {
            plugin: PLUGIN.to_owned()
        })
    );
    let [task_call] = transcript.task_calls(0).try_into().unwrap();
    assert_eq!(
        task_call.not_run,
        Some(NotRunReason::PluginOff {
            plugin: PLUGIN.to_owned()
        })
    );
}

#[tokio::test]
async fn a_plugins_tool_runs_while_the_plugin_is_on() {
    let harness = harness().await;
    harness.runtime.add_plugin(PLUGIN, contributions()).unwrap();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        ECHO,
        &json!({ "activity": "Echoing", "text": "hi" }).to_string(),
    )]));
    harness.model.push_reply(reply("Echoed."));

    let (conversation, run) = start(&harness, "OWNER: echo it").await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let requests = harness.model.requests_for("OWNER:");
    assert!(result(&requests[1], "call_1").starts_with("Echo: "));
    assert_eq!(transcript.tool_calls(0)[0].not_run, None);
}

#[tokio::test]
async fn the_runtime_takes_all_of_a_plugins_contributions_or_none() {
    let harness = harness().await;
    let core_named = PluginContributions {
        tools: vec![PluginTool {
            tool: Arc::new(EchoTool::new()),
            agents: vec!["conversation"],
        }],
        agents: vec![AgentDefinition {
            name: "general",
            ..GENERAL
        }],
        ..PluginContributions::default()
    };

    let refused_agent = harness.runtime.add_plugin(PLUGIN, core_named);
    harness.runtime.add_plugin(PLUGIN, contributions()).unwrap();
    let again = harness
        .runtime
        .add_plugin(PLUGIN, PluginContributions::default());
    let other_tool = harness.runtime.add_plugin(
        "other",
        PluginContributions {
            tools: vec![PluginTool {
                tool: Arc::new(EchoTool::new()),
                agents: vec!["conversation"],
            }],
            ..PluginContributions::default()
        },
    );
    let wrong_kind = harness.runtime.add_plugin(
        "other",
        PluginContributions {
            executors: vec![FakeExecutor::new()],
            ..PluginContributions::default()
        },
    );

    assert_eq!(
        refused_agent,
        Err(ContributionError::DuplicateAgent {
            name: "general".to_owned()
        })
    );
    assert_eq!(
        again,
        Err(ContributionError::PluginRegistered {
            plugin: PLUGIN.to_owned()
        })
    );
    assert_eq!(
        other_tool,
        Err(ContributionError::DuplicateTool {
            name: ECHO.to_owned()
        })
    );
    assert_eq!(
        wrong_kind,
        Err(ContributionError::ExecutorKind {
            plugin: "other".to_owned(),
            kind: "fake".to_owned()
        })
    );
    // The refused plugin left nothing behind: its tool joined the runs only
    // with the second, accepted set.
    harness.model.push_reply(reply("Hello."));
    let (conversation, run) = start(&harness, "OWNER: hello").await;
    harness.wait_for_run(&conversation, &run).await;
    let request = &harness.model.requests_for("OWNER:")[0];
    let names = tool_names(request);
    assert_eq!(names.iter().filter(|name| *name == ECHO).count(), 1);
}
