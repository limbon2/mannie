//! Items that a plugin's tool result cites (ADR-0054, decision 1): the core
//! keeps the plugin's own items with the call, reads them back from the
//! store, leaves out items that break the rules, and deletes them with the
//! conversation.

#![cfg(test)]

use std::sync::{Arc, Mutex};

use futures_util::FutureExt;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::ContextLocation;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::points::context::ContextItem;
use mannie_plugin_sdk::points::tools::{
    AgentSelector, CitedItem, MAX_CITED_ITEMS, MAX_CITED_TEXT_BYTES, PluginTool, Tool, ToolAccess,
    ToolCall, ToolExecution, ToolRefusal, ToolResult, ToolSpec, Tools,
};
use mannie_runtime::testing::{Harness, HarnessOptions, add_plugin, reply, tool_calls};
use mannie_types::chat::{ChatError, StartConversation, SubscribeConversation};
use mannie_types::tools::CitedItemView;
use serde_json::json;

/// The fake plugin's ID and tool.
const PLUGIN: &str = "shell";
const RUN: &str = "shell_run";

/// A plugin's tool that returns the result that a test set.
#[derive(Debug, Default)]
struct Runs {
    result: Mutex<Option<ToolResult>>,
}

impl Tool for Runs {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            RUN,
            "Runs a command.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        )
    }

    fn prepare(&self, _call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        async { Ok(Vec::new()) }.boxed()
    }

    fn execute(&self, _execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let result = self
            .result
            .lock()
            .unwrap()
            .take()
            .unwrap_or_else(|| ToolResult::new("Nothing set."));
        async move { result }.boxed()
    }
}

fn item(key: &str, title: &str, text: &str) -> ContextItem {
    ContextItem::new(key.to_owned(), title.to_owned(), None, text.to_owned())
}

fn view(provider: &str, key: &str, title: &str, text: &str) -> CitedItemView {
    CitedItemView {
        provider: provider.to_owned(),
        key: key.to_owned(),
        title: title.to_owned(),
        location: None,
        text: text.to_owned(),
    }
}

async fn harness_with(result: ToolResult) -> Harness {
    let harness = Harness::with_options(HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    })
    .await;
    let runs = Arc::new(Runs::default());
    *runs.result.lock().unwrap() = Some(result);
    let tool: Arc<dyn Tool> = runs;
    add_plugin(
        &harness.runtime,
        PLUGIN,
        Contributions::new().add::<Tools>(PluginTool::new(tool, AgentSelector::Conversations)),
    )
    .unwrap();
    harness
        .model
        .push_reply(tool_calls(&[("call_1", RUN, "{}")]));
    harness
        .model
        .push_reply(reply("[ls](cite:shell/1.4) shows the files."));
    harness
}

#[tokio::test]
async fn a_call_keeps_the_items_of_its_plugin_that_keep_the_rules() {
    let long = "x".repeat(MAX_CITED_TEXT_BYTES + 10);
    let mut cited = vec![
        CitedItem::new(PLUGIN, item("1.4", "ls", "a.txt\nb.txt\n")),
        CitedItem::new(
            "shell.blocks",
            ContextItem::new(
                "1.5".to_owned(),
                " ".to_owned(),
                Some(ContextLocation::Page { number: 0 }),
                long,
            ),
        ),
        // Another plugin's provider, keys that a link cannot name, and an
        // item without text are left out.
        CitedItem::new("notebooks", item("n1", "Guide", "cited for another plugin")),
        CitedItem::new(PLUGIN, item("1.6#L2", "frag", "a fragment")),
        CitedItem::new(PLUGIN, item("1 6", "space", "a space")),
        CitedItem::new(PLUGIN, item("", "empty", "no key")),
        CitedItem::new(PLUGIN, item("1.7", "blank", " \u{7}\n ")),
    ];
    cited.extend(
        (0..MAX_CITED_ITEMS).map(|number| {
            CitedItem::new(PLUGIN, item(&format!("2.{number}"), "more", "more output"))
        }),
    );
    let harness = harness_with(ToolResult::new("Ran ls.").with_cited(cited)).await;

    let started = harness
        .runtime
        .start_conversation(StartConversation::new("OWNER: which files are there?"))
        .await
        .unwrap();
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;

    let [call] = transcript.tool_calls(0).try_into().unwrap();
    let mut expected = vec![
        view(PLUGIN, "1.4", "ls", "a.txt\nb.txt"),
        view(
            "shell.blocks",
            "1.5",
            "1.5",
            &"x".repeat(MAX_CITED_TEXT_BYTES),
        ),
    ];
    expected.extend(
        (0..MAX_CITED_ITEMS - 2)
            .map(|number| view(PLUGIN, &format!("2.{number}"), "more", "more output")),
    );
    assert_eq!(call.cited, expected);
    // The model read only the output: the items are the transcript's.
    let requests = harness.model.requests();
    let sent = format!("{:?}", requests[1].messages);
    assert!(!sent.contains("a.txt"), "{sent}");

    // The store keeps them with the call.
    let stored = harness.snapshot(&started.conversation_id).await;
    let [stored_call] = stored.tool_calls(0).try_into().unwrap();
    assert_eq!(stored_call.cited, expected);
}

#[tokio::test]
async fn the_items_go_with_their_conversation() {
    let harness = harness_with(
        ToolResult::new("Ran ls.")
            .with_cited(vec![CitedItem::new(PLUGIN, item("1.4", "ls", "a.txt"))]),
    )
    .await;
    let started = harness
        .runtime
        .start_conversation(StartConversation::new("OWNER: which files are there?"))
        .await
        .unwrap();
    let transcript = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    assert_eq!(transcript.tool_calls(0)[0].cited.len(), 1);

    harness
        .runtime
        .delete_conversation(&started.conversation_id)
        .await
        .unwrap();

    let subscribed = harness
        .runtime
        .subscribe(SubscribeConversation {
            conversation_id: started.conversation_id.clone(),
            after_seq: None,
        })
        .await;
    assert!(matches!(subscribed, Err(ChatError::ConversationNotFound)));
}
