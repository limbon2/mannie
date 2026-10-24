//! Tool sources (ADR-0043, decision 2), through fake plugins: a source's
//! sets replace one another and reach the runs that start after them, a run
//! keeps the tools that it started with, a published tool that breaks a rule
//! is left out alone, a published tool gives way to a static tool of its
//! name, a source's tools leave when its stream ends or its plugin stops, and
//! a run offers no more tools than a request takes.

#![cfg(test)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::stream::BoxStream;
use futures_util::{FutureExt, StreamExt};
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::points::tool_sources::{PluginToolSource, ToolSet, ToolSource, ToolSources};
use mannie_plugin_sdk::points::tools::{
    AgentSelector, PluginTool, Tool, ToolAccess, ToolCall, ToolExecution, ToolRefusal, ToolResult,
    ToolSpec, Tools,
};
use mannie_provider::{Content, ModelRequest, Role};
use mannie_runtime::testing::{
    Harness, HarnessOptions, add_plugin, published_tools, remove_plugin, reply, tool_calls,
};
use mannie_types::chat::StartConversation;
use mannie_types::ids::{ConversationId, RunId};
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// The plugin whose source the tests drive.
const PLUGIN: &str = "src";

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

/// The names of the tools of a request.
fn tool_names(request: &ModelRequest) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
}

/// The tools that a run that starts now is offered, through a run marked
/// with `marker`.
async fn offered_now(harness: &Harness, marker: &str) -> Vec<String> {
    harness.model.push_reply_for(marker, reply("Hello."));
    let (conversation, run) = start(harness, &format!("{marker} hello")).await;
    harness.wait_for_run(&conversation, &run).await;
    tool_names(&harness.model.requests_for(marker)[0])
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

/// Waits until the runtime keeps exactly `expected` as the published tools.
async fn until_published(harness: &Harness, expected: &[&str]) {
    let runtime = harness.runtime.clone();
    let expected: Vec<String> = expected.iter().map(|name| (*name).to_owned()).collect();
    tokio::time::timeout(Duration::from_secs(10), async move {
        while published_tools(&runtime) != expected {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the source's set is published");
}

/// A tool that answers with its name, and may hand the source a new set when
/// it runs.
#[derive(Debug)]
struct Named {
    name: String,
    parameters: Value,
    on_call: Option<(mpsc::Sender<ToolSet>, Vec<PluginTool>)>,
}

impl Tool for Named {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            self.name.clone(),
            "Answers with its name.",
            self.parameters.clone(),
        )
        .with_loose_schema()
    }

    fn prepare(&self, _call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        async { Ok(Vec::new()) }.boxed()
    }

    fn execute(&self, _execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        if let Some((sender, next)) = &self.on_call {
            sender.try_send(ToolSet::new(next.clone())).unwrap();
        }
        let output = self.name.clone();
        async move { ToolResult::new(output) }.boxed()
    }
}

/// A tool without arguments under `name`, for the owner's conversations.
fn tool(name: &str) -> PluginTool {
    with_parameters(name, json!({ "type": "object", "properties": {} }))
}

fn with_parameters(name: &str, parameters: Value) -> PluginTool {
    PluginTool::new(
        Arc::new(Named {
            name: name.to_owned(),
            parameters,
            on_call: None,
        }),
        AgentSelector::Conversations,
    )
}

/// A source whose sets come from a channel, which the test drives. The
/// stream ends when the test drops its sender.
#[derive(Debug)]
struct Channel(Mutex<Option<mpsc::Receiver<ToolSet>>>);

impl ToolSource for Channel {
    fn sets(&self) -> BoxStream<'static, ToolSet> {
        let receiver = self
            .0
            .lock()
            .unwrap()
            .take()
            .expect("the core takes the stream once");
        futures_util::stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|set| (set, receiver))
        })
        .boxed()
    }
}

/// A plugin `plugin` with one source, whose sender the test keeps.
fn with_channel(plugin: &str) -> (Contributions, mpsc::Sender<ToolSet>) {
    let (sender, receiver) = mpsc::channel(16);
    let source = PluginToolSource::new(plugin, Arc::new(Channel(Mutex::new(Some(receiver)))));
    (Contributions::new().add::<ToolSources>(source), sender)
}

#[tokio::test]
async fn each_set_replaces_the_one_before_and_reaches_the_runs_that_start_after_it() {
    let harness = harness().await;
    let (contributions, sender) = with_channel(PLUGIN);
    add_plugin(&harness.runtime, PLUGIN, contributions).unwrap();

    sender
        .try_send(ToolSet::new(vec![tool("src_one"), tool("src_two")]))
        .unwrap();
    until_published(&harness, &["src_one", "src_two"]).await;
    let first = offered_now(&harness, "FIRST:").await;
    sender
        .try_send(ToolSet::new(vec![tool("src_three")]))
        .unwrap();
    until_published(&harness, &["src_three"]).await;
    let second = offered_now(&harness, "SECOND:").await;

    assert!(first.contains(&"src_one".to_owned()), "{first:?}");
    assert!(first.contains(&"src_two".to_owned()), "{first:?}");
    assert!(!second.contains(&"src_one".to_owned()), "{second:?}");
    assert!(second.contains(&"src_three".to_owned()), "{second:?}");
}

#[tokio::test]
async fn a_run_keeps_the_published_tools_that_it_started_with() {
    let harness = harness().await;
    let (contributions, sender) = with_channel(PLUGIN);
    add_plugin(&harness.runtime, PLUGIN, contributions).unwrap();
    // The tool replaces the source's set with one without itself when it
    // runs, in the middle of the reply that calls it twice.
    let replacing = PluginTool::new(
        Arc::new(Named {
            name: "src_once".to_owned(),
            parameters: json!({ "type": "object", "properties": {} }),
            on_call: Some((sender.clone(), vec![tool("src_after")])),
        }),
        AgentSelector::Conversations,
    );
    sender.try_send(ToolSet::new(vec![replacing])).unwrap();
    until_published(&harness, &["src_once"]).await;
    let call = json!({ "activity": "Calling" }).to_string();
    harness
        .model
        .push_reply(tool_calls(&[("call_1", "src_once", &call)]));
    harness
        .model
        .push_reply(tool_calls(&[("call_2", "src_once", &call)]));
    harness.model.push_reply(reply("Done."));

    let (conversation, run) = start(&harness, "OWNER: call it twice").await;
    harness.wait_for_run(&conversation, &run).await;
    until_published(&harness, &["src_after"]).await;
    let later = offered_now(&harness, "LATER:").await;

    let requests = harness.model.requests_for("OWNER:");
    assert_eq!(result(&requests[1], "call_1"), "src_once");
    assert_eq!(result(&requests[2], "call_2"), "src_once");
    assert!(tool_names(&requests[2]).contains(&"src_once".to_owned()));
    assert!(!later.contains(&"src_once".to_owned()), "{later:?}");
    assert!(later.contains(&"src_after".to_owned()), "{later:?}");
}

#[tokio::test]
async fn a_published_tool_that_breaks_a_rule_is_left_out_alone() {
    let harness = harness().await;
    let (contributions, sender) = with_channel(PLUGIN);
    add_plugin(&harness.runtime, PLUGIN, contributions).unwrap();

    sender
        .try_send(ToolSet::new(vec![
            tool("other_tool"),
            tool("src_Upper"),
            with_parameters(
                "src_activity",
                json!({ "type": "object", "properties": { "activity": { "type": "string" } } }),
            ),
            with_parameters("src_not_an_object", json!(true)),
            tool("src_kept"),
            tool("src_kept"),
        ]))
        .unwrap();

    until_published(&harness, &["src_kept"]).await;
}

#[tokio::test]
async fn a_published_tool_gives_way_to_a_static_tool_of_its_name() {
    let harness = harness().await;
    let (contributions, sender) = with_channel(PLUGIN);
    add_plugin(&harness.runtime, PLUGIN, contributions).unwrap();
    sender
        .try_send(ToolSet::new(vec![tool("src_shared"), tool("src_own")]))
        .unwrap();
    until_published(&harness, &["src_shared", "src_own"]).await;

    // Another plugin starts with a static tool of the same name, which the
    // published one never keeps from starting.
    add_plugin(
        &harness.runtime,
        "other",
        Contributions::new().add::<Tools>(tool("src_shared")),
    )
    .unwrap();
    let offered = offered_now(&harness, "OFFERED:").await;
    // A set that arrives later leaves the taken name out at once.
    sender
        .try_send(ToolSet::new(vec![tool("src_shared"), tool("src_new")]))
        .unwrap();
    until_published(&harness, &["src_new"]).await;

    let shared = offered.iter().filter(|name| *name == "src_shared").count();
    assert_eq!(shared, 1, "{offered:?}");
    assert!(offered.contains(&"src_own".to_owned()), "{offered:?}");
}

#[tokio::test]
async fn a_sources_tools_leave_when_its_stream_ends_or_its_plugin_stops() {
    let harness = harness().await;
    let (contributions, sender) = with_channel(PLUGIN);
    add_plugin(&harness.runtime, PLUGIN, contributions).unwrap();
    sender
        .try_send(ToolSet::new(vec![tool("src_one")]))
        .unwrap();
    until_published(&harness, &["src_one"]).await;

    drop(sender);
    until_published(&harness, &[]).await;

    let (contributions, sender) = with_channel("again");
    add_plugin(&harness.runtime, "again", contributions).unwrap();
    sender
        .try_send(ToolSet::new(vec![tool("again_one")]))
        .unwrap();
    until_published(&harness, &["again_one"]).await;
    assert!(remove_plugin(&harness.runtime, "again"));
    assert_eq!(published_tools(&harness.runtime), Vec::<String>::new());
    // A set that the stopped plugin's source sends is never published, also
    // while its drain is still being aborted.
    let sent_after_stop = sender
        .try_send(ToolSet::new(vec![tool("again_two")]))
        .is_ok();
    let offered = offered_now(&harness, "AFTER:").await;

    assert!(
        published_tools(&harness.runtime).is_empty(),
        "sent after the stop: {sent_after_stop}"
    );
    assert!(
        !offered.iter().any(|name| name.starts_with("again_")),
        "{offered:?}"
    );
}

/// A source whose first set is ready at once.
#[derive(Debug)]
struct Ready(Vec<PluginTool>);

impl ToolSource for Ready {
    fn sets(&self) -> BoxStream<'static, ToolSet> {
        let set = ToolSet::new(self.0.clone());
        futures_util::stream::once(async move { set })
            .chain(futures_util::stream::pending())
            .boxed()
    }
}

#[tokio::test]
async fn a_set_that_is_ready_at_the_start_is_published_before_the_start_ends() {
    let harness = harness().await;
    let source = PluginToolSource::new(PLUGIN, Arc::new(Ready(vec![tool("src_cached")])));

    add_plugin(
        &harness.runtime,
        PLUGIN,
        Contributions::new().add::<ToolSources>(source),
    )
    .unwrap();

    assert_eq!(published_tools(&harness.runtime), ["src_cached"]);
}

#[tokio::test]
async fn a_run_offers_no_more_tools_than_a_request_takes() {
    let harness = harness().await;
    // Two plugins publish 64 tools each, more than a request takes beside
    // the core's tools.
    for plugin in ["left", "right"] {
        let tools = (0..64)
            .map(|index| tool(&format!("{plugin}_{index:02}")))
            .collect();
        let source = PluginToolSource::new(plugin, Arc::new(Ready(tools)));
        add_plugin(
            &harness.runtime,
            plugin,
            Contributions::new().add::<ToolSources>(source),
        )
        .unwrap();
    }

    let offered = offered_now(&harness, "MANY:").await;

    assert!(offered.len() <= 128, "{}", offered.len());
    assert!(offered.contains(&"read_file".to_owned()));
    assert!(offered.contains(&"left_00".to_owned()));
    assert!(
        !offered.contains(&"right_63".to_owned()),
        "the last published tools are left out first"
    );
}
