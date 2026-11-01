//! A plugin's optional features in the runtime (ADR-0055, decision 6): a
//! feature's contributions are a group of their own beside its plugin's,
//! with a generation of their own, so turning the feature off drops its
//! entries alone, a run's call of its tool after that does not run and names
//! the feature, and a feature contributes no task executor.

#![cfg(test)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::FutureExt;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::host::{FEATURE_POINT, PLUGIN_REGISTERED};
use mannie_plugin_sdk::points::executors::Executors;
use mannie_plugin_sdk::points::sections::{Section, SectionBody, Sections};
use mannie_plugin_sdk::points::tools::{
    AgentSelector, PluginTool, Tool, ToolAccess, ToolCall, ToolExecution, ToolRefusal, ToolResult,
    ToolSpec, Tools,
};
use mannie_provider::testing::Step;
use mannie_provider::{Content, ModelRequest, Role};
use mannie_runtime::testing::{
    FakeExecutor, Harness, HarnessOptions, add_feature, add_plugin, reply, tool_calls,
    turn_off_feature,
};
use mannie_runtime::texts;
use mannie_types::chat::StartConversation;
use mannie_types::ids::{ConversationId, RunId};
use mannie_types::tools::{NotRunReason, ToolCallStatus, ToolOutcome};
use serde_json::json;

/// The plugin of the tests.
const PLUGIN: &str = "notes";

/// The plugin's feature.
const FEATURE: &str = "history";

/// The feature's tool.
const RECALL: &str = "recall_history";

/// The plugin's own section, which stays while the feature goes off.
const SECTION: &str = "The notes plugin keeps the owner's notes.";

/// A tool of the feature that answers with its arguments and counts its
/// executions.
#[derive(Debug, Default)]
struct Recall {
    executions: Mutex<u32>,
}

impl Tool for Recall {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            RECALL,
            "Recalls the history.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        )
    }

    fn prepare(&self, _call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        async { Ok(Vec::new()) }.boxed()
    }

    fn execute(&self, _execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        *self.executions.lock().unwrap() += 1;
        async { ToolResult::new("Recalled.") }.boxed()
    }
}

/// The plugin's own contributions: a section.
fn plugin_contributions() -> Contributions {
    Contributions::new().add::<Sections>(Section::new(
        "notes",
        500,
        SectionBody::Fixed(SECTION.to_owned()),
    ))
}

/// The feature's contributions: its tool.
fn feature_contributions(recall: &Arc<Recall>) -> Contributions {
    let tool: Arc<dyn Tool> = Arc::<Recall>::clone(recall);
    Contributions::new().add::<Tools>(PluginTool::new(tool, AgentSelector::Conversations))
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

fn tool_names(request: &ModelRequest) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
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
                ..
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

fn recall_call(call_id: &str) -> Vec<Step> {
    tool_calls(&[(
        call_id,
        RECALL,
        &json!({ "activity": "Recalling" }).to_string(),
    )])
}

#[tokio::test]
async fn a_features_tool_runs_while_the_feature_is_on_as_a_tool_of_its_plugin() {
    let harness = harness().await;
    let recall = Arc::new(Recall::default());
    add_plugin(&harness.runtime, PLUGIN, plugin_contributions()).unwrap();
    add_feature(
        &harness.runtime,
        PLUGIN,
        FEATURE,
        feature_contributions(&recall),
    )
    .unwrap();
    harness.model.push_reply(recall_call("call_1"));
    harness.model.push_reply(reply("Recalled."));

    let (conversation, run) = start(&harness, "OWNER: recall it").await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let requests = harness.model.requests_for("OWNER:");
    assert!(tool_names(&requests[0]).contains(&RECALL.to_owned()));
    assert!(requests[0].instructions.contains(SECTION));
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
    assert_eq!(*recall.executions.lock().unwrap(), 1);
}

#[tokio::test]
async fn turning_a_feature_off_drops_its_entries_and_leaves_its_plugins() {
    let harness = harness().await;
    let recall = Arc::new(Recall::default());
    add_plugin(&harness.runtime, PLUGIN, plugin_contributions()).unwrap();
    add_feature(
        &harness.runtime,
        PLUGIN,
        FEATURE,
        feature_contributions(&recall),
    )
    .unwrap();

    let dropped = turn_off_feature(&harness.runtime, PLUGIN, FEATURE, Duration::from_secs(1)).await;
    let again = turn_off_feature(&harness.runtime, PLUGIN, FEATURE, Duration::from_secs(1)).await;
    harness.model.push_reply(reply("Hello."));
    let (conversation, run) = start(&harness, "OWNER: hello").await;
    harness.wait_for_run(&conversation, &run).await;

    assert!(dropped);
    assert!(!again, "a feature that is off has nothing left to drop");
    let request = &harness.model.requests_for("OWNER:")[0];
    assert!(!tool_names(request).contains(&RECALL.to_owned()));
    assert!(request.instructions.contains(SECTION));
}

#[tokio::test]
async fn a_call_of_a_feature_turned_off_during_a_run_does_not_run_and_names_the_feature() {
    let harness = harness().await;
    let recall = Arc::new(Recall::default());
    add_plugin(&harness.runtime, PLUGIN, plugin_contributions()).unwrap();
    add_feature(
        &harness.runtime,
        PLUGIN,
        FEATURE,
        feature_contributions(&recall),
    )
    .unwrap();
    let mut held = vec![Step::Delay(Duration::from_millis(300))];
    held.extend(recall_call("call_1"));
    harness.model.push_reply(held);
    harness.model.push_reply(reply("The feature is off."));

    let (conversation, run) = start(&harness, "OWNER: recall it").await;
    until_requested(&harness, "OWNER:").await;
    turn_off_feature(&harness.runtime, PLUGIN, FEATURE, Duration::from_secs(1)).await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let requests = harness.model.requests_for("OWNER:");
    assert_eq!(
        result(&requests[1], "call_1"),
        texts::tool_feature_off(PLUGIN, FEATURE)
    );
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(
        call.not_run,
        Some(NotRunReason::PluginOff {
            plugin: PLUGIN.to_owned(),
            feature: Some(FEATURE.to_owned()),
        })
    );
    assert_eq!(*recall.executions.lock().unwrap(), 0);
}

#[tokio::test]
async fn a_feature_turned_off_and_on_during_a_run_is_off_for_the_runs_call() {
    let harness = harness().await;
    let before = Arc::new(Recall::default());
    let after = Arc::new(Recall::default());
    add_feature(
        &harness.runtime,
        PLUGIN,
        FEATURE,
        feature_contributions(&before),
    )
    .unwrap();
    let mut held = vec![Step::Delay(Duration::from_millis(300))];
    held.extend(recall_call("call_1"));
    harness.model.push_reply(held);
    harness.model.push_reply(reply("The feature restarted."));

    let (conversation, run) = start(&harness, "OWNER: recall it").await;
    until_requested(&harness, "OWNER:").await;
    turn_off_feature(&harness.runtime, PLUGIN, FEATURE, Duration::from_secs(1)).await;
    add_feature(
        &harness.runtime,
        PLUGIN,
        FEATURE,
        feature_contributions(&after),
    )
    .unwrap();
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::NotRun);
    assert_eq!(*before.executions.lock().unwrap(), 0);
    assert_eq!(*after.executions.lock().unwrap(), 0);
}

#[tokio::test]
async fn a_features_contributions_are_refused_twice_and_never_hold_a_task_executor() {
    let harness = harness().await;
    let recall = Arc::new(Recall::default());
    add_plugin(&harness.runtime, PLUGIN, plugin_contributions()).unwrap();
    add_feature(
        &harness.runtime,
        PLUGIN,
        FEATURE,
        feature_contributions(&recall),
    )
    .unwrap();

    let twice = add_feature(
        &harness.runtime,
        PLUGIN,
        FEATURE,
        Contributions::new().add::<Sections>(Section::new(
            "notes.history",
            510,
            SectionBody::Fixed("History.".to_owned()),
        )),
    )
    .unwrap_err();
    let executor = add_feature(
        &harness.runtime,
        PLUGIN,
        "tasks",
        Contributions::new().add::<Executors>(FakeExecutor::new()),
    )
    .unwrap_err();

    assert_eq!(twice.code, PLUGIN_REGISTERED);
    assert_eq!(executor.code, FEATURE_POINT);
}
