//! Attached context in the runtime (ADR-0046, ADR-0049, ADR-0050, ADR-0051;
//! plugins/notebooks/docs/README.md): a plugin's context provider through
//! the point `context-providers`, the attachments that it describes, a
//! conversation's home and the mentions that attach, the gathering before a
//! reply's first model call for the home and for what a message names, with
//! the owner's earlier messages, its failures, its timeout and Stop, a retry
//! that reuses what was gathered, the outlines in the instructions, what the
//! model reads of the excerpts of the newest messages and of older ones, and
//! a plugin's tool that is offered where its attachments are, whose calls
//! carry their keys, also a child's (ADR-0052).

#![cfg(test)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::ContextLocation;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::host::{ID_NOT_OWNED, ID_TAKEN};
use mannie_plugin_sdk::points::context::{
    ContextItem, ContextProvider, ContextProviders, ContextRequest, PluginContextProvider,
};
use mannie_plugin_sdk::points::tools::{
    AgentSelector, PluginTool, Tool, ToolAccess, ToolCall, ToolExecution, ToolOffering,
    ToolRefusal, ToolResult, ToolSpec, Tools,
};
use mannie_provider::{Content, ModelRequest, ProviderError, Role};
use mannie_runtime::testing::{
    Harness, HarnessOptions, SeenPart, add_plugin, failure, fast_config, remove_plugin, reply,
    tool_calls,
};
use mannie_runtime::{RuntimeConfig, texts};
use mannie_types::chat::{ChatError, SendMessage, StartConversation, StartedConversation};
use mannie_types::context::{Attachment, AttachmentRef, ChangeAttachment, ContextView};
use mannie_types::ids::{ConversationId, ProjectId};
use mannie_types::mentions::MentionRef;
use mannie_types::run::{RunActivity, RunFailure, RunState};
use mannie_types::transcript::PartContent;
use serde_json::json;

/// What the fake provider does when it gathers.
#[derive(Debug, Clone)]
enum Behavior {
    /// Gives these items.
    Gives(Vec<ContextItem>),
    /// Fails with this code.
    Fails(&'static str),
    /// Never answers.
    Hangs,
}

/// A plugin's provider that knows the notebooks `n1`, Physics, and `n2`,
/// Chemistry, outlines them, and records what it was asked.
#[derive(Debug)]
struct Library {
    behavior: Mutex<Behavior>,
    requests: Mutex<Vec<ContextRequest>>,
}

impl Library {
    fn new(behavior: Behavior) -> Arc<Self> {
        Arc::new(Self {
            behavior: Mutex::new(behavior),
            requests: Mutex::new(Vec::new()),
        })
    }

    fn set(&self, behavior: Behavior) {
        *self.behavior.lock().unwrap_or_else(PoisonError::into_inner) = behavior;
    }

    fn requests(&self) -> Vec<ContextRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn label_of(key: &str) -> Option<&'static str> {
    HashMap::from([("n1", "Physics"), ("n2", "Chemistry")])
        .get(key)
        .copied()
}

impl ContextProvider for Library {
    fn describe(&self, key: String) -> BoxFuture<'static, Result<String, String>> {
        let label = label_of(&key)
            .map(str::to_owned)
            .ok_or_else(|| "notebook_not_found".to_owned());
        Box::pin(std::future::ready(label))
    }

    fn gather(
        &self,
        request: ContextRequest,
    ) -> BoxFuture<'static, Result<Vec<ContextItem>, String>> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let behavior = self
            .behavior
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        match behavior {
            Behavior::Gives(items) => Box::pin(std::future::ready(Ok(items))),
            Behavior::Fails(code) => Box::pin(std::future::ready(Err(code.to_owned()))),
            Behavior::Hangs => Box::pin(std::future::pending()),
        }
    }

    fn outline(&self, keys: Vec<String>) -> BoxFuture<'static, Result<String, String>> {
        let lines: Vec<String> = keys
            .iter()
            .filter_map(|key| label_of(key))
            .map(|label| format!("- Notebook \"{label}\": Optics.pdf (source s1, 12 pages)"))
            .collect();
        Box::pin(std::future::ready(Ok(lines.join("\n"))))
    }
}

fn passage(key: &str, title: &str, page: u32, text: &str) -> ContextItem {
    ContextItem::new(
        key.to_owned(),
        title.to_owned(),
        Some(ContextLocation::Page { number: page }),
        text.to_owned(),
    )
}

fn optics() -> ContextItem {
    passage(
        "s1:0",
        "Optics.pdf",
        3,
        "Light bends when it changes medium.",
    )
}

/// What the model reads of [`optics`] with its cite link.
const OPTICS_EXCERPT: &str = "[1] Optics.pdf, page 3 · cite: [Optics.pdf p. 3](cite:notebooks/s1:0)\nLight bends when it changes medium.";

fn contributions(library: &Arc<Library>) -> Contributions {
    let provider: Arc<dyn ContextProvider> = Arc::<Library>::clone(library);
    Contributions::new().add::<ContextProviders>(PluginContextProvider::new("notebooks", provider))
}

/// A harness with the plugin `notebooks`, whose provider behaves as asked,
/// and which waits 200 ms for a provider.
async fn setup(behavior: Behavior) -> (Harness, Arc<Library>) {
    setup_waiting(behavior, Duration::from_millis(200)).await
}

/// A harness with the plugin `notebooks`, which waits `timeout` for a
/// provider.
async fn setup_waiting(behavior: Behavior, timeout: Duration) -> (Harness, Arc<Library>) {
    let harness = Harness::with_options(HarnessOptions {
        config: RuntimeConfig {
            gather_timeout: timeout,
            ..fast_config()
        },
        ..HarnessOptions::default()
    })
    .await;
    let library = Library::new(behavior);
    add_plugin(&harness.runtime, "notebooks", contributions(&library)).unwrap();
    (harness, library)
}

fn notebook(key: &str) -> AttachmentRef {
    AttachmentRef {
        provider: "notebooks".to_owned(),
        key: key.to_owned(),
    }
}

fn change(conversation: &ConversationId, provider: &str, key: &str) -> ChangeAttachment {
    ChangeAttachment {
        conversation_id: conversation.clone(),
        attachment: AttachmentRef {
            provider: provider.to_owned(),
            key: key.to_owned(),
        },
    }
}

async fn started(harness: &Harness, start: StartConversation) -> StartedConversation {
    let started = harness.runtime.start_conversation(start).await.unwrap();
    harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    started
}

/// A conversation with the notebooks `keys` attached, whose messages gather
/// only what they name.
async fn start(harness: &Harness, text: &str, keys: &[&str]) -> StartedConversation {
    let attachments = keys.iter().map(|key| notebook(key)).collect();
    started(
        harness,
        StartConversation::new(text).with_attachments(attachments),
    )
    .await
}

/// A conversation that belongs to the notebook `key`, whose every message
/// gathers from it.
async fn start_home(harness: &Harness, text: &str, key: &str) -> StartedConversation {
    started(
        harness,
        StartConversation::new(text).with_home(Some(notebook(key))),
    )
    .await
}

async fn send(harness: &Harness, conversation: &ConversationId, text: &str) {
    let started = harness
        .runtime
        .send_message(SendMessage::new(conversation.clone(), text))
        .await
        .unwrap();
    harness.wait_for_run(conversation, &started.run_id).await;
}

/// Sends a message that names the notebook `key` with "@".
async fn send_naming(harness: &Harness, conversation: &ConversationId, text: &str, key: &str) {
    let mention = MentionRef {
        provider: "notebooks".to_owned(),
        key: key.to_owned(),
    };
    let started = harness
        .runtime
        .send_message(SendMessage::new(conversation.clone(), text).with_mentions(vec![mention]))
        .await
        .unwrap();
    harness.wait_for_run(conversation, &started.run_id).await;
}

async fn attachments(harness: &Harness, conversation: &ConversationId) -> Vec<Attachment> {
    harness
        .store
        .conversation_summary(conversation)
        .await
        .unwrap()
        .unwrap()
        .attachments
}

/// The texts of each owner message of a request, in order.
fn owner_messages(request: &ModelRequest) -> Vec<Vec<String>> {
    request
        .messages
        .iter()
        .filter(|message| message.role == Role::User)
        .map(|message| {
            message
                .content
                .iter()
                .map(|content| match content {
                    Content::Text(text) => text.clone(),
                    other => format!("{other:?}"),
                })
                .collect()
        })
        .collect()
}

fn contexts(parts: &[SeenPart]) -> Vec<ContextView> {
    parts
        .iter()
        .filter_map(|part| match &part.content {
            PartContent::Context { context } => Some(context.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_reply_reads_what_the_home_gives_and_the_instructions_outline_it() {
    let (harness, library) = setup(Behavior::Gives(vec![optics()])).await;
    harness.model.push_reply(reply(
        "It changes speed [Optics.pdf p. 3](cite:notebooks/s1:0).",
    ));

    let started = start_home(&harness, "Why does light bend?", "n1").await;
    let transcript = harness.snapshot(&started.conversation_id).await;
    let request = harness.model.reply_requests().pop().unwrap();
    let summary = harness
        .store
        .conversation_summary(&started.conversation_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        attachments(&harness, &started.conversation_id).await,
        [Attachment {
            provider: "notebooks".to_owned(),
            key: "n1".to_owned(),
            label: "Physics".to_owned(),
        }]
    );
    assert_eq!(summary.home, Some(notebook("n1")));
    assert_eq!(
        owner_messages(&request),
        [vec![
            "Why does light bend?".to_owned(),
            format!("{}\n\n{OPTICS_EXCERPT}", texts::CONTEXT_HEADING),
        ]]
    );
    assert!(
        request
            .instructions
            .contains("This conversation belongs to the owner's \"Physics\""),
        "{}",
        request.instructions
    );
    assert!(
        request
            .instructions
            .contains("- Notebook \"Physics\": Optics.pdf (source s1, 12 pages)")
    );
    let asked = library.requests();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].keys, ["n1"]);
    assert_eq!(asked[0].message, "Why does light bend?");
    assert_eq!(asked[0].earlier_messages, Vec::<String>::new());
    assert_eq!(asked[0].conversation_id, started.conversation_id);
    assert!(asked[0].max_bytes > 0);
    let gathered = contexts(&transcript.parts(0));
    assert_eq!(gathered.len(), 1);
    assert_eq!(gathered[0].attachments[0].label, "Physics");
    assert_eq!(gathered[0].items[0].number, 1);
    assert_eq!(gathered[0].failure, None);
}

#[tokio::test]
async fn a_regular_conversation_gathers_only_for_a_message_that_names_an_attachment() {
    let (harness, library) = setup(Behavior::Gives(vec![optics()])).await;
    for answer in ["One.", "Two.", "Three."] {
        harness.model.push_reply(reply(answer));
    }

    let started = start(&harness, "One", &["n1"]).await;
    let conversation = started.conversation_id;
    send_naming(&harness, &conversation, "Two @Chemistry", "n2").await;
    send(&harness, &conversation, "Three").await;
    let requests = harness.model.reply_requests();
    let latest = |request: &ModelRequest| owner_messages(request).pop().unwrap();

    // Only the message that names Chemistry gathered, and only from it.
    let asked = library.requests();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].keys, ["n2"]);
    assert_eq!(asked[0].message, "Two @Chemistry");
    assert_eq!(latest(&requests[0]), ["One"]);
    assert_eq!(
        latest(&requests[1]),
        [
            "Two @Chemistry".to_owned(),
            texts::mention_attached("Chemistry"),
            format!("{}\n\n{OPTICS_EXCERPT}", texts::CONTEXT_HEADING),
        ]
    );
    assert_eq!(latest(&requests[2]), ["Three"]);
    let labels: Vec<String> = attachments(&harness, &conversation)
        .await
        .into_iter()
        .map(|attachment| attachment.label)
        .collect();
    assert_eq!(labels, ["Physics", "Chemistry"]);
    // Every request of a conversation with attachments says how to use them.
    assert!(
        requests[0]
            .instructions
            .contains("The owner attached the sources listed below")
    );
    assert!(
        !requests[0]
            .instructions
            .contains("excerpts of the sources found")
    );
    assert!(
        requests[1]
            .instructions
            .contains("excerpts of the sources found")
    );
}

#[tokio::test]
async fn a_follow_up_carries_the_owner_s_earlier_messages() {
    let (harness, library) = setup(Behavior::Gives(Vec::new())).await;
    for answer in ["One.", "Two.", "Three."] {
        harness.model.push_reply(reply(answer));
    }

    let started = start_home(&harness, "How does WALLIX handle SSH?", "n1").await;
    send(&harness, &started.conversation_id, "And the proxy?").await;
    send(&harness, &started.conversation_id, "Elaborate").await;

    let earlier: Vec<Vec<String>> = library
        .requests()
        .into_iter()
        .map(|request| request.earlier_messages)
        .collect();
    assert_eq!(
        earlier,
        [
            Vec::<String>::new(),
            vec!["How does WALLIX handle SSH?".to_owned()],
            vec![
                "And the proxy?".to_owned(),
                "How does WALLIX handle SSH?".to_owned()
            ],
        ]
    );
}

#[tokio::test]
async fn a_home_stays_attached_and_keeps_its_conversation_out_of_projects() {
    let (harness, _library) = setup(Behavior::Gives(Vec::new())).await;
    harness.model.push_reply(reply("Hello."));
    let started = start_home(&harness, "Hello", "n1").await;
    let conversation = started.conversation_id;

    let detached = harness
        .runtime
        .detach_context(change(&conversation, "notebooks", "n1"))
        .await;
    let in_project = harness
        .runtime
        .start_conversation(
            StartConversation::new("Hi")
                .with_home(Some(notebook("n1")))
                .with_project(Some(
                    ProjectId::parse("0199e0f2-0000-7000-8000-00000000abcd").unwrap(),
                )),
        )
        .await;

    assert_eq!(detached, Err(ChatError::HomeAttachment));
    assert_eq!(in_project.unwrap_err(), ChatError::HomeConversation);
    assert_eq!(attachments(&harness, &conversation).await.len(), 1);
}

#[tokio::test]
async fn attaching_asks_the_provider_and_detaching_works_while_its_plugin_is_off() {
    let (harness, _library) = setup(Behavior::Gives(Vec::new())).await;
    harness.model.push_reply(reply("Hello."));
    let started = start(&harness, "Hello", &[]).await;
    let conversation = started.conversation_id;

    harness
        .runtime
        .attach_context(change(&conversation, "notebooks", "n2"))
        .await
        .unwrap();
    let unknown = harness
        .runtime
        .attach_context(change(&conversation, "notebooks", "n9"))
        .await;
    let nobody = harness
        .runtime
        .attach_context(change(&conversation, "wiki", "w1"))
        .await;
    let attached = attachments(&harness, &conversation).await;
    remove_plugin(&harness.runtime, "notebooks");
    let while_off = harness
        .runtime
        .attach_context(change(&conversation, "notebooks", "n1"))
        .await;
    harness
        .runtime
        .detach_context(change(&conversation, "notebooks", "n2"))
        .await
        .unwrap();

    assert_eq!(
        attached
            .iter()
            .map(|attachment| attachment.label.as_str())
            .collect::<Vec<_>>(),
        ["Chemistry"]
    );
    assert_eq!(
        unknown,
        Err(ChatError::ContextRefused {
            provider: "notebooks".to_owned(),
            reason: "notebook_not_found".to_owned(),
        })
    );
    assert_eq!(
        nobody,
        Err(ChatError::ContextUnavailable {
            provider: "wiki".to_owned()
        })
    );
    assert_eq!(
        while_off,
        Err(ChatError::ContextUnavailable {
            provider: "notebooks".to_owned()
        })
    );
    assert_eq!(attachments(&harness, &conversation).await, Vec::new());
}

#[tokio::test]
async fn a_new_conversation_whose_key_a_provider_refuses_is_not_stored() {
    let (harness, library) = setup(Behavior::Gives(Vec::new())).await;

    let refused = harness
        .runtime
        .start_conversation(StartConversation::new("Hi").with_attachments(vec![notebook("n9")]))
        .await;
    let refused_home = harness
        .runtime
        .start_conversation(StartConversation::new("Hi").with_home(Some(notebook("n9"))))
        .await;

    for refusal in [refused, refused_home] {
        assert_eq!(
            refusal.unwrap_err(),
            ChatError::ContextRefused {
                provider: "notebooks".to_owned(),
                reason: "notebook_not_found".to_owned(),
            }
        );
    }
    assert_eq!(
        harness.store.list_conversations().await.unwrap(),
        Vec::new()
    );
    assert_eq!(library.requests(), Vec::new());
}

#[tokio::test]
async fn a_provider_that_fails_takes_too_long_or_is_off_gives_a_part_that_says_why() {
    let (harness, library) = setup(Behavior::Fails("server_unreachable")).await;
    for answer in ["One.", "Two.", "Three."] {
        harness.model.push_reply(reply(answer));
    }

    let started = start_home(&harness, "One", "n1").await;
    let conversation = started.conversation_id;
    library.set(Behavior::Hangs);
    send(&harness, &conversation, "Two").await;
    remove_plugin(&harness.runtime, "notebooks");
    send(&harness, &conversation, "Three").await;
    let transcript = harness.snapshot(&conversation).await;
    let requests = harness.model.reply_requests();

    let latest = |request: &ModelRequest| owner_messages(request).pop().unwrap();
    assert_eq!(
        latest(&requests[0]),
        [
            "One",
            "[Mannie could not search the attached \"Physics\": server_unreachable.]"
        ]
    );
    assert_eq!(
        latest(&requests[1]),
        [
            "Two",
            "[The attached \"Physics\" were not searched: the search took too long.]"
        ]
    );
    assert_eq!(
        latest(&requests[2]),
        [
            "Three",
            "[The attached \"Physics\" were not searched, because their plugin is off.]"
        ]
    );
    assert!(
        requests[2]
            .instructions
            .contains("\"Physics\": what they hold could not be read (plugin_off)"),
        "{}",
        requests[2].instructions
    );
    let failures: Vec<Option<String>> = (0..3)
        .flat_map(|position| contexts(&transcript.parts(position)))
        .map(|context| context.failure)
        .collect();
    assert_eq!(
        failures,
        [
            Some("server_unreachable".to_owned()),
            Some("timed_out".to_owned()),
            Some("plugin_off".to_owned())
        ]
    );
}

#[tokio::test]
async fn a_retry_reuses_what_the_first_run_gathered() {
    let (harness, library) = setup(Behavior::Gives(vec![optics()])).await;
    harness
        .model
        .push_reply(failure(ProviderError::Permanent(RunFailure::BadRequest {
            status: 400,
        })));
    harness.model.push_reply(reply("It changes speed."));
    let started = start_home(&harness, "Why does light bend?", "n1").await;

    let retried = harness
        .runtime
        .retry_reply(&started.conversation_id)
        .await
        .unwrap();
    harness
        .wait_for_run(&started.conversation_id, &retried.run_id)
        .await;
    let requests = harness.model.reply_requests();

    assert_eq!(library.requests().len(), 1);
    assert_eq!(owner_messages(&requests[0]), owner_messages(&requests[1]));
    let transcript = harness.snapshot(&started.conversation_id).await;
    assert_eq!(contexts(&transcript.parts(0)).len(), 1);
}

#[tokio::test]
async fn stop_ends_a_gathering_and_a_retry_gathers_again() {
    // The provider hangs far longer than the test waits before it stops.
    let (harness, library) = setup_waiting(Behavior::Hangs, Duration::from_secs(600)).await;
    let started = harness
        .runtime
        .start_conversation(
            StartConversation::new("Why does light bend?").with_home(Some(notebook("n1"))),
        )
        .await
        .unwrap();
    harness
        .wait_until(&started.conversation_id, |transcript| {
            transcript.run(&started.run_id).is_some_and(|run| {
                matches!(run.activity, Some(RunActivity::GatheringContext { .. }))
            })
        })
        .await;

    harness.runtime.stop_reply(&started.conversation_id).await;
    let stopped = harness
        .wait_for_run(&started.conversation_id, &started.run_id)
        .await;
    library.set(Behavior::Gives(vec![optics()]));
    harness.model.push_reply(reply("It changes speed."));
    let retried = harness
        .runtime
        .retry_reply(&started.conversation_id)
        .await
        .unwrap();
    let transcript = harness
        .wait_for_run(&started.conversation_id, &retried.run_id)
        .await;

    assert_eq!(stopped.run_state(&started.run_id), Some(RunState::Stopped));
    assert_eq!(library.requests().len(), 2);
    assert_eq!(contexts(&transcript.parts(0)).len(), 1);
    assert_eq!(harness.model.reply_requests().len(), 1);
}

#[tokio::test]
async fn only_the_two_newest_messages_with_excerpts_send_them_in_full() {
    let (harness, library) = setup(Behavior::Gives(vec![optics()])).await;
    for answer in ["One.", "Two.", "Three."] {
        harness.model.push_reply(reply(answer));
    }
    let started = start_home(&harness, "One", "n1").await;
    library.set(Behavior::Gives(vec![
        passage("s2:0", "Waves.md", 1, "Waves carry energy."),
        passage("s2:1", "Waves.md", 2, "Sound is a wave."),
    ]));
    send(&harness, &started.conversation_id, "Two").await;
    library.set(Behavior::Gives(Vec::new()));
    send(&harness, &started.conversation_id, "Three").await;

    let third = harness.model.reply_requests().pop().unwrap();
    let messages = owner_messages(&third);

    // The third message holds no excerpts, so the first two keep theirs in
    // full: they are the two newest that hold any.
    assert_eq!(
        messages[0][1],
        format!("{}\n\n{OPTICS_EXCERPT}", texts::CONTEXT_HEADING)
    );
    assert!(messages[1][1].contains(
        "[2] Waves.md, page 2 · cite: [Waves.md p. 2](cite:notebooks/s2:1)\nSound is a wave."
    ));
    assert_eq!(
        messages[2][1],
        "[The attached \"Physics\" hold nothing that applies to this message.]"
    );
    library.set(Behavior::Gives(vec![optics()]));
    harness.model.push_reply(reply("Four."));
    send(&harness, &started.conversation_id, "Four").await;
    let fourth = harness.model.reply_requests().pop().unwrap();
    assert_eq!(
        owner_messages(&fourth)[0][1],
        "[Excerpt [1] from the attached \"Physics\" went with this message and is not repeated.]"
    );
}

/// A tool of the plugin `notebooks`, offered where its attachments are, which
/// keeps the keys that each of its calls carried.
#[derive(Debug, Default)]
struct KeysTool {
    seen: Mutex<Vec<Vec<String>>>,
}

/// The tool's name.
const KEYS_TOOL: &str = "notes_keys";

impl Tool for KeysTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            KEYS_TOOL,
            "Lists the keys of the attached notebooks.",
            json!({
                "additionalProperties": false,
                "properties": {},
                "required": [],
                "type": "object",
            }),
        )
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call.attachments);
        Box::pin(std::future::ready(Ok(Vec::new())))
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let output = execution.attachments.join(",");
        Box::pin(std::future::ready(ToolResult::new(output)))
    }
}

/// The names of the tools that a request offered.
fn offered(request: &ModelRequest) -> Vec<String> {
    request.tools.iter().map(|tool| tool.name.clone()).collect()
}

#[tokio::test]
async fn a_tool_where_attachments_are_reaches_their_keys_also_from_a_child() {
    let harness = Harness::with_options(HarnessOptions {
        use_tools: true,
        ..HarnessOptions::default()
    })
    .await;
    let library = Library::new(Behavior::Gives(Vec::new()));
    let keys = Arc::new(KeysTool::default());
    let tool: Arc<dyn Tool> = Arc::<KeysTool>::clone(&keys);
    let contributions = contributions(&library).add::<Tools>(
        PluginTool::new(tool, AgentSelector::Every)
            .offered_where(ToolOffering::WhereAttached("notebooks".to_owned())),
    );
    add_plugin(&harness.runtime, "notebooks", contributions).unwrap();
    let start_task = json!({
        "activity": "Delegating",
        "title": "List the keys",
        "agent": "general",
        "instruction": "CHILD-K: list the keys",
        "background": false,
    })
    .to_string();
    harness.model.push_reply_for("PLAIN:", reply("Hello."));
    harness
        .model
        .push_reply_for("HOME:", tool_calls(&[("call_1", KEYS_TOOL, "{}")]));
    harness.model.push_reply_for(
        "HOME:",
        tool_calls(&[("call_2", "start_task", &start_task)]),
    );
    harness.model.push_reply_for("HOME:", reply("Done."));
    for steps in [
        tool_calls(&[("child_1", KEYS_TOOL, "{}")]),
        reply("listed"),
        reply("Report: listed."),
    ] {
        harness.model.push_reply_for("CHILD-K", steps);
    }

    started(&harness, StartConversation::new("PLAIN: hello")).await;
    let home = started(
        &harness,
        StartConversation::new("HOME: which keys?").with_home(Some(notebook("n1"))),
    )
    .await;
    let transcript = harness.snapshot(&home.conversation_id).await;
    let task = transcript.tasks(0)[0].clone();
    harness
        .wait_until(&task.conversation_id, |child| {
            !child.tool_calls(0).is_empty()
        })
        .await;

    assert!(!offered(&harness.model.requests_for("PLAIN:")[0]).contains(&KEYS_TOOL.to_owned()));
    assert!(offered(&harness.model.requests_for("HOME:")[0]).contains(&KEYS_TOOL.to_owned()));
    assert!(offered(&harness.model.requests_for("CHILD-K")[0]).contains(&KEYS_TOOL.to_owned()));
    // The parent's call and the child's carry the parent's notebook.
    assert_eq!(
        *keys.seen.lock().unwrap_or_else(PoisonError::into_inner),
        [vec!["n1".to_owned()], vec!["n1".to_owned()]]
    );
}

#[tokio::test]
async fn a_plugin_s_providers_need_ids_of_their_own() {
    let (harness, _library) = setup(Behavior::Gives(Vec::new())).await;
    let provider: Arc<dyn ContextProvider> = Library::new(Behavior::Gives(Vec::new()));
    let contributions = |ids: &[&str]| {
        ids.iter().fold(Contributions::new(), |contributions, id| {
            contributions
                .add::<ContextProviders>(PluginContextProvider::new(*id, Arc::clone(&provider)))
        })
    };
    // The point and the code of a refusal.
    let refused = |plugin: &str, ids: &[&str]| {
        add_plugin(&harness.runtime, plugin, contributions(ids))
            .err()
            .map(|refusal| (refusal.point, refusal.code))
    };

    let foreign = refused("tickets", &["notebooks"]);
    let twice = refused("wiki", &["wiki", "wiki"]);
    let own = refused("wiki", &["wiki", "wiki.pages"]);

    assert_eq!(foreign, Some(("context-providers", ID_NOT_OWNED)));
    assert_eq!(twice, Some(("context-providers", ID_TAKEN)));
    assert_eq!(own, None);
}
