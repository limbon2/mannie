//! What plugins contribute to the runtime while they are on (ADR-0030,
//! decisions 5 and 8; ADR-0034, decision 6), through fake plugins: every
//! extension point reaches the runtime's consumer and stopping the plugin
//! drops them all, the runtime takes all of a plugin's contributions or none,
//! a point without a consumer refuses the plugin, entries follow their
//! point's rule of IDs, a tool reaches the agents that its selector names and
//! runs through the core's tool loop, a section's source renders from what
//! the runtime tells it of the run, a run keeps what it started with, and its
//! calls into a plugin that stopped, or stopped and started again, do not
//! run.

#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::FutureExt;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::contracts::{ImageBytes, ImageFormat, MentionEntry, MentionSummary};
use mannie_plugin_sdk::contributions::{Contributions, ExtensionPoint, IdRule, PointInfo};
use mannie_plugin_sdk::host::{
    ID_NOT_OWNED, ID_TAKEN, INVALID_NAME, KIND_NOT_PLUGIN, NO_CONSUMER, PLUGIN_REGISTERED,
    RESERVED_ARGUMENT, TOO_MANY_ENTRIES,
};
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::points;
use mannie_plugin_sdk::points::agents::{AgentBudgets, AgentSpec, Agents, Requester};
use mannie_plugin_sdk::points::executors::{Executors, TaskExecutor};
use mannie_plugin_sdk::points::images::{ImageSource, ImageSources, SourcedImage};
use mannie_plugin_sdk::points::mentions::{
    MentionContent, MentionContext, MentionProvider, Mentions, ProviderSuggestions, RenderedMention,
};
use mannie_plugin_sdk::points::sections::{
    Section, SectionBody, SectionContext, SectionSource, Sections,
};
use mannie_plugin_sdk::points::tools::{
    AgentSelector, PluginTool, Tool, ToolAccess, ToolCall, ToolExecution, ToolRefusal, ToolResult,
    ToolSpec, Tools,
};
use mannie_provider::testing::Step;
use mannie_provider::{Content, ModelRequest, Role};
use mannie_runtime::testing::{
    FAKE_EXECUTOR_KIND, FAKE_EXECUTOR_SECTION, FAKE_EXECUTOR_TOOL, FakeExecutor, Harness,
    HarnessOptions, add_plugin, remove_plugin, reply, tool_calls,
};
use mannie_runtime::{GENERAL, texts};
use mannie_tools::{MIN_RESULT_LIMIT_BYTES, RESULT_LIMIT_BYTES, result_limit};
use mannie_types::chat::{ChatError, StartConversation};
use mannie_types::ids::{ConversationId, RunId};
use mannie_types::images::ImageRef;
use mannie_types::mentions::{MentionPlace, SuggestMentions};
use mannie_types::provider::context_budget;
use mannie_types::run::RunState;
use mannie_types::tools::{NotRunReason, ToolCallStatus, ToolOutcome, UpdateToolsSettings};
use serde_json::{Value, json};

/// The fake plugin's ID.
const PLUGIN: &str = "echo";

/// The fake plugin's tool.
const ECHO: &str = "echo_text";

/// The fake plugin's section.
const SECTION: &str = "The echo plugin repeats text with echo_text.";

/// The fake plugin's child agent.
const HELPER: &str = "helper";

/// The mention provider of the plugin of every point, whose ID is the
/// executor's kind and more.
const NOTES: &str = "fake.notes";

/// The image source of the plugin of every point.
const SHOTS: &str = "fake.shots";

/// The schema of a tool's arguments: one text.
fn text_argument() -> Value {
    json!({
        "additionalProperties": false,
        "properties": { "text": { "type": "string" } },
        "required": ["text"],
        "type": "object",
    })
}

/// A tool that answers with its arguments, and keeps the calls that it was
/// prepared and executed with.
#[derive(Debug, Default)]
struct EchoTool {
    calls: Mutex<Vec<ToolCall>>,
    executions: Mutex<Vec<ToolExecution>>,
}

impl EchoTool {
    /// Whether the core prepared or executed a call of the tool.
    fn was_called(&self) -> bool {
        !self.calls.lock().unwrap().is_empty() || !self.executions.lock().unwrap().is_empty()
    }
}

impl Tool for EchoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(ECHO, "Repeats a text.", text_argument())
    }

    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        self.calls.lock().unwrap().push(call);
        async { Ok(Vec::new()) }.boxed()
    }

    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let output = format!("Echo: {}", execution.arguments);
        self.executions.lock().unwrap().push(execution);
        async move { ToolResult::new(output) }.boxed()
    }
}

/// A tool of the plugin, offered to the agents of the owner's conversations.
fn for_conversations(tool: Arc<dyn Tool>) -> PluginTool {
    PluginTool::new(tool, AgentSelector::Conversations)
}

/// A tool under a name and with a schema, which answers every call with the
/// same output.
#[derive(Debug)]
struct Answering {
    name: String,
    parameters: Value,
    output: String,
}

impl Tool for Answering {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            self.name.clone(),
            "Answers with its output.",
            self.parameters.clone(),
        )
    }

    fn prepare(&self, _call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        async { Ok(Vec::new()) }.boxed()
    }

    fn execute(&self, _execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let output = self.output.clone();
        async move { ToolResult::new(output) }.boxed()
    }
}

/// A tool without arguments, under `name`, for the agents that `offered_to`
/// names, which answers with its name.
fn named(name: &str, offered_to: AgentSelector) -> PluginTool {
    with_schema(
        name,
        json!({ "type": "object", "properties": {} }),
        offered_to,
    )
}

/// A tool under `name` with the schema `parameters`, which answers with its
/// name.
fn with_schema(name: &str, parameters: Value, offered_to: AgentSelector) -> PluginTool {
    PluginTool::new(
        Arc::new(Answering {
            name: name.to_owned(),
            parameters,
            output: name.to_owned(),
        }),
        offered_to,
    )
}

/// A tool that refuses the arguments of every call, and notes whether it
/// was executed all the same.
#[derive(Debug, Default)]
struct Refusing {
    executed: AtomicBool,
}

impl Tool for Refusing {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(ECHO, "Refuses every text.", text_argument())
    }

    fn prepare(&self, _call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        async { Err(ToolRefusal::new("the text is empty")) }.boxed()
    }

    fn execute(&self, _execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        self.executed.store(true, Ordering::SeqCst);
        async { ToolResult::new("Echo: nothing") }.boxed()
    }
}

/// The name of the tool whose execution never ends.
const ENDLESS: &str = "echo_forever";

/// A tool whose execution never ends, which notes when its execution started
/// and when its future was dropped.
#[derive(Debug, Default)]
struct Endless {
    started: Arc<AtomicBool>,
    dropped: Arc<AtomicBool>,
}

/// Sets its flag when it is dropped, which shows when the future that holds
/// it was dropped.
#[derive(Debug)]
struct SetOnDrop(Arc<AtomicBool>);

impl Drop for SetOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

impl Tool for Endless {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            ENDLESS,
            "Repeats nothing, forever.",
            json!({ "type": "object", "properties": {} }),
        )
    }

    fn prepare(&self, _call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
        async { Ok(Vec::new()) }.boxed()
    }

    fn execute(&self, _execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
        let started = Arc::clone(&self.started);
        let guard = SetOnDrop(Arc::clone(&self.dropped));
        async move {
            started.store(true, Ordering::SeqCst);
            let never = futures_util::future::pending::<ToolResult>().await;
            // The guard lives as long as the future, which only a drop ends.
            drop(guard);
            never
        }
        .boxed()
    }
}

/// A child agent of the plugin with this name, which does what the core's
/// general child does.
const fn like_general(name: &'static str) -> AgentSpec {
    AgentSpec::new(
        name,
        Requester::Agent,
        GENERAL.instructions,
        GENERAL.tools,
        AgentBudgets::new(
            GENERAL.max_continuation_notes,
            GENERAL.max_tool_rounds,
            GENERAL.max_tool_calls,
            GENERAL.max_tasks,
        ),
    )
}

/// The fake plugin's child agent.
const HELPER_AGENT: AgentSpec = like_general(HELPER);

/// The fake plugin's section, at a position after the core's.
fn echo_section(id: &'static str, text: &str) -> Section {
    Section::new(id, 900, SectionBody::Fixed(text.to_owned()))
}

fn contributions() -> Contributions {
    with_echo(Arc::new(EchoTool::default()))
}

/// The fake plugin's contributions, with `echo` as its tool.
fn with_echo(echo: Arc<EchoTool>) -> Contributions {
    Contributions::new()
        .add::<Tools>(for_conversations(echo))
        .add::<Agents>(HELPER_AGENT)
        .add::<Sections>(echo_section("echo", SECTION))
}

/// A mention provider that is offered everywhere and suggests one note.
#[derive(Debug)]
struct Notes {
    id: &'static str,
}

/// What the note of [`Notes`] gives the model.
struct NoteContent;

impl MentionContent for NoteContent {
    fn render(&self, limit: usize) -> RenderedMention {
        let text = "The notes.".chars().take(limit).collect();
        let label = "Notes".to_owned();
        let variant = "note".to_owned();
        let summary = MentionSummary {
            code: "note".to_owned(),
            values: BTreeMap::new(),
        };
        RenderedMention::new(text, label, variant, summary)
    }
}

impl MentionProvider for Notes {
    fn id(&self) -> &str {
        self.id
    }

    fn offered(&self, _context: &MentionContext) -> bool {
        true
    }

    fn suggest<'a>(
        &'a self,
        _context: &'a MentionContext,
        _query: &'a str,
        _limit: usize,
    ) -> BoxFuture<'a, Result<ProviderSuggestions, String>> {
        let entry = MentionEntry {
            key: "note".to_owned(),
            label: "note".to_owned(),
            token: "@note".to_owned(),
            variant: "note".to_owned(),
            warning: None,
            expands: false,
        };
        futures_util::future::ready(Ok(ProviderSuggestions::new(vec![entry], false))).boxed()
    }

    fn resolve<'a>(
        &'a self,
        _context: &'a MentionContext,
        _key: &'a str,
        _max_bytes: usize,
    ) -> BoxFuture<'a, Result<Box<dyn MentionContent>, String>> {
        let content: Box<dyn MentionContent> = Box::new(NoteContent);
        futures_util::future::ready(Ok(content)).boxed()
    }
}

fn notes(id: &'static str) -> Arc<dyn MentionProvider> {
    Arc::new(Notes { id })
}

/// An image source that holds one PNG under the key `one`.
#[derive(Debug)]
struct Shots {
    id: &'static str,
}

impl ImageSource for Shots {
    fn id(&self) -> &str {
        self.id
    }

    fn resolve<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<SourcedImage, String>> {
        let image = if key == "one" {
            let name = "one.png".to_owned();
            let format = ImageFormat::Png;
            let width = 800;
            let height = 600;
            let data = ImageBytes::from(b"\x89PNG\r\n\x1a\n1".to_vec());
            let thumbnail_format = ImageFormat::Jpeg;
            let thumbnail = ImageBytes::from(b"\xff\xd8\xff1".to_vec());
            Ok(SourcedImage::new(
                name,
                format,
                width,
                height,
                data,
                thumbnail_format,
                thumbnail,
            ))
        } else {
            Err("not_found".to_owned())
        };
        futures_util::future::ready(image).boxed()
    }

    fn release(&self, _keys: &[String]) {}
}

fn shots(id: &'static str) -> Arc<dyn ImageSource> {
    Arc::new(Shots { id })
}

fn executor() -> Arc<dyn TaskExecutor> {
    FakeExecutor::new()
}

/// The contributions of a plugin to every point of SDK 1, with the image
/// source `source`. The plugin's ID is the fake executor's kind.
fn every_point(source: &'static str) -> Contributions {
    contributions()
        .add::<Executors>(executor())
        .add::<Mentions>(notes(NOTES))
        .add::<ImageSources>(shots(source))
}

/// A section's source that tells the model what the runtime told it of the
/// run, and leaves the section out of a run without tools.
#[derive(Debug)]
struct RunFacts;

impl SectionSource for RunFacts {
    fn render(&self, context: &SectionContext) -> Option<String> {
        context.uses_tools.then(|| {
            format!(
                "FACTS: agent {}, answers to {:?}, tasks {}, full access {}",
                context.agent, context.requester, context.offers_tasks, context.full_access
            )
        })
    }
}

fn with_run_facts() -> Contributions {
    Contributions::new().add::<Sections>(Section::new(
        "echo",
        900,
        SectionBody::Source(Arc::new(RunFacts)),
    ))
}

/// A point of a later SDK, which no consumer of this runtime takes.
enum Things {}

impl ExtensionPoint for Things {
    const NAME: &'static str = "things";
    const MAJOR: u16 = 1;
    const IDS: IdRule = IdRule::Global;
    type Entry = &'static str;

    fn id(entry: &Self::Entry) -> &str {
        entry
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
        .start_conversation(StartConversation::new(text))
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

/// The points of the fake plugin of [`every_point`] that the runtime offers
/// now, in the order of the SDK's points: whether a run that starts gets the
/// plugin's tool, its child agent, its section and its executor's tool with
/// its section, whether the "@" list has a group of its mention provider, and
/// whether a message can hold an image of its source. Each is read through
/// runs, the "@" list and a message with an image, marked with `marker`.
async fn reached(harness: &Harness, marker: &str) -> Vec<&'static str> {
    let first = format!("{marker}:");
    harness.model.push_reply_for(&first, reply("Hello."));
    let (conversation, run) = start(harness, &format!("{first} hello")).await;
    harness.wait_for_run(&conversation, &run).await;
    let request = &harness.model.requests_for(&first)[0];
    let tools = tool_names(request);
    let suggestions = harness
        .runtime
        .suggest_mentions(SuggestMentions::new(
            MentionPlace::NewConversation { project_id: None },
            "",
        ))
        .await
        .unwrap();
    let with_image = format!("{marker} IMAGE:");
    harness.model.push_reply_for(&with_image, reply("Seen."));
    let image = harness
        .runtime
        .start_conversation(
            StartConversation::new(format!("{with_image} look")).with_images(vec![ImageRef {
                source: SHOTS.to_owned(),
                key: "one".to_owned(),
            }]),
        )
        .await;
    let images = match image {
        Ok(started) => {
            harness
                .wait_for_run(&started.conversation_id, &started.run_id)
                .await;
            true
        }
        Err(ChatError::ImageUnavailable { image_source }) => {
            assert_eq!(image_source, SHOTS);
            false
        }
        Err(other) => panic!("a message with an image failed: {other}"),
    };
    let offered = [
        (Tools::NAME, tools.contains(&ECHO.to_owned())),
        (
            Agents::NAME,
            child_agents(request).contains(&HELPER.to_owned()),
        ),
        (Sections::NAME, request.instructions.contains(SECTION)),
        (
            Executors::NAME,
            tools.contains(&FAKE_EXECUTOR_TOOL.to_owned())
                && request.instructions.contains(FAKE_EXECUTOR_SECTION),
        ),
        (
            Mentions::NAME,
            suggestions
                .groups
                .iter()
                .any(|group| group.provider == NOTES),
        ),
        (ImageSources::NAME, images),
    ];
    offered
        .into_iter()
        .filter(|(_point, reached)| *reached)
        .map(|(point, _reached)| point)
        .collect()
}

/// The names of every point of the SDK, in their order.
fn every_point_name() -> Vec<&'static str> {
    points::ALL.iter().map(|point| point.name).collect()
}

#[tokio::test]
async fn every_point_reaches_its_consumer_and_stopping_drops_them_all() {
    let harness = harness().await;
    add_plugin(&harness.runtime, FAKE_EXECUTOR_KIND, every_point(SHOTS)).unwrap();

    let on = reached(&harness, "ON").await;
    let removed = remove_plugin(&harness.runtime, FAKE_EXECUTOR_KIND);
    let off = reached(&harness, "OFF").await;

    assert_eq!(on, every_point_name());
    assert!(removed);
    assert_eq!(off, Vec::<&str>::new());
    assert!(
        !remove_plugin(&harness.runtime, FAKE_EXECUTOR_KIND),
        "it was off already"
    );
}

#[tokio::test]
async fn a_refused_image_source_leaves_no_tool_agent_section_executor_or_mention_behind() {
    let harness = harness().await;
    add_plugin(
        &harness.runtime,
        "other",
        Contributions::new().add::<ImageSources>(shots("other.shots")),
    )
    .unwrap();
    let fake = PluginId::parse(FAKE_EXECUTOR_KIND).unwrap();
    let [consumer] = harness.runtime.contribution_consumers().try_into().unwrap();

    // The coordinator refuses a source that is not the plugin's before the
    // runtime sees any entry.
    let foreign = add_plugin(
        &harness.runtime,
        FAKE_EXECUTOR_KIND,
        every_point("other.shots"),
    )
    .unwrap_err();
    // The runtime's consumer checks its points in order and the image
    // sources last, so its own refusal of a source whose ID another plugin
    // has comes after the tool, the agent, the section, the executor and the
    // mention provider passed. The coordinator would have refused the ID
    // first, so the test hands the entries to the consumer as the coordinator
    // does after its checks.
    let checked = consumer.check(&fake, &every_point("other.shots"));
    let taken = consumer.add(&fake, every_point("other.shots"));
    let left = reached(&harness, "REFUSED").await;
    add_plugin(&harness.runtime, FAKE_EXECUTOR_KIND, every_point(SHOTS)).unwrap();
    let accepted = reached(&harness, "ACCEPTED").await;

    assert_eq!(
        (foreign.point, foreign.code),
        ("image-sources", ID_NOT_OWNED)
    );
    let checked = checked.unwrap_err();
    let taken = taken.unwrap_err();
    assert_eq!((checked.point, checked.code), ("image-sources", ID_TAKEN));
    assert_eq!((taken.point, taken.code), ("image-sources", ID_TAKEN));
    assert!(taken.detail.contains("other.shots"), "{}", taken.detail);
    assert_eq!(left, Vec::<&str>::new());
    assert_eq!(accepted, every_point_name());
}

#[tokio::test]
async fn a_point_without_a_consumer_refuses_the_plugin_and_names_the_point() {
    let harness = harness().await;

    let refused = add_plugin(
        &harness.runtime,
        PLUGIN,
        contributions().add::<Things>("thing"),
    )
    .unwrap_err();
    harness.model.push_reply(reply("Hello."));
    let (conversation, run) = start(&harness, "OWNER: hello").await;
    harness.wait_for_run(&conversation, &run).await;

    assert_eq!((refused.point, refused.code), ("things", NO_CONSUMER));
    assert!(refused.detail.contains("things"), "{}", refused.detail);
    // The plugin's entries of the points that have a consumer did not join
    // either.
    assert!(!remove_plugin(&harness.runtime, PLUGIN));
    let request = &harness.model.requests_for("OWNER:")[0];
    assert!(!tool_names(request).contains(&ECHO.to_owned()));
    assert!(!request.instructions.contains(SECTION));
}

#[tokio::test]
async fn entries_follow_their_points_id_rules() {
    let harness = harness().await;
    let note = |id: &'static str| Contributions::new().add::<Mentions>(notes(id));
    let source = |id: &'static str| Contributions::new().add::<ImageSources>(shots(id));
    let tool = |name: &str| Contributions::new().add::<Tools>(named(name, AgentSelector::Every));
    let agent = |spec: AgentSpec| Contributions::new().add::<Agents>(spec);
    let section = |id: &'static str| {
        Contributions::new().add::<Sections>(echo_section(id, &format!("REFUSED: {id}")))
    };
    let executors = |count: usize| {
        (0..count).fold(Contributions::new(), |contributions, _index| {
            contributions.add::<Executors>(executor())
        })
    };
    // Owned: the plugin's ID, or the plugin's ID, a dot and more. PluginKind:
    // exactly the plugin's ID, so one entry at most. Global, against the
    // core: a system tool, a task tool, an agent and a section.
    let alone = [
        ("fake", note("fakeish"), Some(("mentions", ID_NOT_OWNED))),
        ("fake", note("fake."), Some(("mentions", ID_NOT_OWNED))),
        (
            "fake",
            note("other.notes"),
            Some(("mentions", ID_NOT_OWNED)),
        ),
        (
            "fake",
            source("fakeish"),
            Some(("image-sources", ID_NOT_OWNED)),
        ),
        (
            "fake",
            source("fake."),
            Some(("image-sources", ID_NOT_OWNED)),
        ),
        ("other", executors(1), Some(("executors", KIND_NOT_PLUGIN))),
        (
            FAKE_EXECUTOR_KIND,
            executors(2),
            Some(("executors", TOO_MANY_ENTRIES)),
        ),
        (PLUGIN, tool("read_file"), Some(("tools", ID_TAKEN))),
        (PLUGIN, tool("start_task"), Some(("tools", ID_TAKEN))),
        (
            PLUGIN,
            agent(like_general("general")),
            Some(("agents", ID_TAKEN)),
        ),
        (PLUGIN, section("base"), Some(("sections", ID_TAKEN))),
    ];
    // Global, against the entries of the echo plugin, a second start of the
    // echo plugin, and owned IDs and a kind of the plugin's own.
    let beside_echo = [
        (
            "other",
            Contributions::new().add::<Tools>(for_conversations(Arc::new(EchoTool::default()))),
            Some(("tools", ID_TAKEN)),
        ),
        ("other", agent(HELPER_AGENT), Some(("agents", ID_TAKEN))),
        ("other", section("echo"), Some(("sections", ID_TAKEN))),
        (
            PLUGIN,
            tool("echo_more"),
            Some(("tools", PLUGIN_REGISTERED)),
        ),
        (
            FAKE_EXECUTOR_KIND,
            note("fake.notes")
                .add::<ImageSources>(shots("fake"))
                .add::<Executors>(executor()),
            None,
        ),
    ];

    let mut outcomes = Vec::new();
    for (plugin, contributions, expected) in alone {
        let refused = add_plugin(&harness.runtime, plugin, contributions).err();
        outcomes.push((
            refused.map(|refusal| (refusal.point, refusal.code)),
            expected,
        ));
    }
    add_plugin(&harness.runtime, PLUGIN, contributions()).unwrap();
    for (plugin, contributions, expected) in beside_echo {
        let refused = add_plugin(&harness.runtime, plugin, contributions).err();
        outcomes.push((
            refused.map(|refusal| (refusal.point, refusal.code)),
            expected,
        ));
    }
    harness.model.push_reply(reply("Hello."));
    let (conversation, run) = start(&harness, "OWNER: hello").await;
    harness.wait_for_run(&conversation, &run).await;

    for (index, (outcome, expected)) in outcomes.iter().enumerate() {
        assert_eq!(outcome, expected, "case {index}");
    }
    // The refused sets left nothing behind: the run has the echo tool and
    // section once, and none of the refused names or texts.
    let request = &harness.model.requests_for("OWNER:")[0];
    let names = tool_names(request);
    assert_eq!(names.iter().filter(|name| *name == ECHO).count(), 1);
    assert!(!names.contains(&"echo_more".to_owned()), "{names:?}");
    assert_eq!(request.instructions.matches(SECTION).count(), 1);
    assert!(
        !request.instructions.contains("REFUSED:"),
        "{}",
        request.instructions
    );
    assert!(!remove_plugin(&harness.runtime, "other"));
}

#[tokio::test]
async fn the_runtimes_consumers_cover_every_point_of_the_sdk() {
    let harness = Harness::start().await;

    let covered: Vec<PointInfo> = harness
        .runtime
        .contribution_consumers()
        .iter()
        .flat_map(|consumer| consumer.points())
        .collect();

    for point in points::ALL {
        assert!(covered.contains(point), "{point:?} has no consumer");
    }
}

#[tokio::test]
async fn a_plugins_section_source_sees_the_agent_whether_the_run_uses_tools_and_full_access() {
    let harness = harness().await;
    add_plugin(&harness.runtime, PLUGIN, with_run_facts()).unwrap();
    let start_child = json!({
        "activity": "Starting",
        "title": "Look",
        "agent": "explore",
        "instruction": "CHILD: look around",
        "background": false,
    })
    .to_string();
    harness
        .model
        .push_reply(tool_calls(&[("call_1", "start_task", &start_child)]));
    harness.model.push_reply(reply("The child looked."));
    harness.model.push_reply_for("CHILD:", reply("Looked."));
    harness
        .model
        .push_reply_for("CHILD:", reply("Report: looked."));
    let (conversation, run) = start(&harness, "OWNER: look").await;
    harness.wait_for_run(&conversation, &run).await;
    harness
        .runtime
        .update_tools_settings(UpdateToolsSettings {
            allow_changes_in_workspace: false,
            command_environment: Vec::new(),
            full_access_for_new_conversations: true,
            wake_on_command_end: true,
        })
        .await
        .unwrap();
    harness.model.push_reply(reply("In full access."));
    let (conversation, run) = start(&harness, "FULL: look").await;
    harness.wait_for_run(&conversation, &run).await;
    let without_tools = Harness::start().await;
    add_plugin(&without_tools.runtime, PLUGIN, with_run_facts()).unwrap();
    without_tools.model.push_reply(reply("Hello."));
    let (conversation, run) = start(&without_tools, "PLAIN: hello").await;
    without_tools.wait_for_run(&conversation, &run).await;

    let parent = &harness.model.requests_for("OWNER:")[0];
    let child = &harness.model.requests_for("CHILD:")[0];
    let full = &harness.model.requests_for("FULL:")[0];
    let plain = &without_tools.model.requests_for("PLAIN:")[0];
    assert!(
        parent
            .instructions
            .contains("FACTS: agent conversation, answers to Owner, tasks true, full access false"),
        "{}",
        parent.instructions
    );
    assert!(
        child
            .instructions
            .contains("FACTS: agent explore, answers to Agent, tasks false, full access false"),
        "{}",
        child.instructions
    );
    assert!(
        full.instructions
            .contains("FACTS: agent conversation, answers to Owner, tasks true, full access true"),
        "{}",
        full.instructions
    );
    assert!(
        !plain.instructions.contains("FACTS:"),
        "{}",
        plain.instructions
    );
}

#[tokio::test]
async fn a_run_keeps_what_it_started_with_and_its_calls_into_a_stopped_plugin_do_not_run() {
    let harness = harness().await;
    add_plugin(&harness.runtime, PLUGIN, contributions()).unwrap();
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
    remove_plugin(&harness.runtime, PLUGIN);
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
async fn a_tool_of_a_plugin_turned_off_and_on_during_a_run_does_not_run() {
    let harness = harness().await;
    let before = Arc::new(EchoTool::default());
    let after = Arc::new(EchoTool::default());
    add_plugin(
        &harness.runtime,
        PLUGIN,
        with_echo(Arc::<EchoTool>::clone(&before)),
    )
    .unwrap();
    let echo = |call_id: &str| {
        tool_calls(&[(
            call_id,
            ECHO,
            &json!({ "activity": "Echoing", "text": "hi" }).to_string(),
        )])
    };
    let mut held = vec![Step::Delay(Duration::from_millis(300))];
    held.extend(echo("call_1"));
    harness.model.push_reply_for("FIRST:", held);
    harness
        .model
        .push_reply_for("FIRST:", reply("The plugin restarted."));

    let (conversation, run) = start(&harness, "FIRST: echo it").await;
    until_requested(&harness, "FIRST:").await;
    // The plugin stops and starts again while the model writes the call.
    assert!(remove_plugin(&harness.runtime, PLUGIN));
    add_plugin(
        &harness.runtime,
        PLUGIN,
        with_echo(Arc::<EchoTool>::clone(&after)),
    )
    .unwrap();
    let transcript = harness.wait_for_run(&conversation, &run).await;
    // A run that starts now gets the new start's tool, which runs.
    harness.model.push_reply_for("NEXT:", echo("call_2"));
    harness.model.push_reply_for("NEXT:", reply("Echoed."));
    let (conversation, run) = start(&harness, "NEXT: echo it").await;
    let next = harness.wait_for_run(&conversation, &run).await;

    let requests = harness.model.requests_for("FIRST:");
    assert_eq!(
        result(&requests[1], "call_1"),
        texts::tool_plugin_off(PLUGIN)
    );
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(
        call.not_run,
        Some(NotRunReason::PluginOff {
            plugin: PLUGIN.to_owned()
        })
    );
    assert!(!before.was_called(), "the stopped start's tool never runs");
    let [ran] = next.tool_calls(0).try_into().unwrap();
    assert_eq!(ran.not_run, None);
    assert_eq!(ran.status, ToolCallStatus::Completed);
    let [executed] = after.executions.lock().unwrap().clone().try_into().unwrap();
    assert_eq!(executed.conversation_id, conversation);
}

#[tokio::test]
async fn a_plugins_tool_runs_while_the_plugin_is_on() {
    let harness = harness().await;
    let echo = Arc::new(EchoTool::default());
    add_plugin(
        &harness.runtime,
        PLUGIN,
        with_echo(Arc::<EchoTool>::clone(&echo)),
    )
    .unwrap();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        ECHO,
        &json!({ "activity": "Echoing", "text": "hi" }).to_string(),
    )]));
    harness.model.push_reply(reply("Echoed."));

    let (conversation, run) = start(&harness, "OWNER: echo it").await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let requests = harness.model.requests_for("OWNER:");
    // The tool reads the arguments without the activity, which the call
    // carries for the owner.
    assert_eq!(result(&requests[1], "call_1"), r#"Echo: {"text":"hi"}"#);
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.not_run, None);
    assert_eq!(call.status, ToolCallStatus::Completed);
    assert_eq!(call.activity.as_deref(), Some("Echoing"));
    let result = call.result.unwrap();
    assert_eq!(
        result.outcome,
        ToolOutcome::Plugin {
            plugin: PLUGIN.to_owned(),
            data: None,
            failure: None,
        }
    );
    assert_eq!(result.output, r#"Echo: {"text":"hi"}"#);
    let [prepared] = echo.calls.lock().unwrap().clone().try_into().unwrap();
    let [executed] = echo.executions.lock().unwrap().clone().try_into().unwrap();
    assert_eq!(prepared.arguments, r#"{"text":"hi"}"#);
    assert_eq!(prepared.conversation_id, conversation);
    assert_eq!(executed.arguments, prepared.arguments);
    assert_eq!(executed.conversation_id, conversation);
}

#[tokio::test]
async fn each_agent_selector_offers_the_tool_to_its_agents() {
    let harness = harness().await;
    add_plugin(
        &harness.runtime,
        PLUGIN,
        Contributions::new()
            .add::<Tools>(named("for_conversations", AgentSelector::Conversations))
            .add::<Tools>(named("for_child_agents", AgentSelector::ChildAgents))
            .add::<Tools>(named("for_every_agent", AgentSelector::Every))
            .add::<Tools>(named(
                "for_the_helper",
                AgentSelector::Agents(vec![HELPER.to_owned(), "missing".to_owned()]),
            ))
            .add::<Tools>(named("for_no_agent", AgentSelector::Agents(Vec::new())))
            .add::<Agents>(HELPER_AGENT),
    )
    .unwrap();
    let start_child = |agent: &str, instruction: &str| {
        json!({
            "activity": "Starting",
            "title": "Look",
            "agent": agent,
            "instruction": instruction,
            "background": false,
        })
        .to_string()
    };
    harness.model.push_reply(tool_calls(&[
        (
            "call_1",
            "start_task",
            &start_child("explore", "EXPLORE: look around"),
        ),
        ("call_2", "start_task", &start_child(HELPER, "HELPER: help")),
    ]));
    harness.model.push_reply(reply("The children looked."));
    for marker in ["EXPLORE:", "HELPER:"] {
        harness.model.push_reply_for(marker, reply("Looked."));
        harness
            .model
            .push_reply_for(marker, reply("Report: looked."));
    }

    let (conversation, run) = start(&harness, "OWNER: look").await;
    harness.wait_for_run(&conversation, &run).await;

    let offered = |marker: &str| -> Vec<String> {
        tool_names(&harness.model.requests_for(marker)[0])
            .into_iter()
            .filter(|name| name.starts_with("for_"))
            .collect()
    };
    assert_eq!(offered("OWNER:"), ["for_conversations", "for_every_agent"]);
    assert_eq!(offered("EXPLORE:"), ["for_child_agents", "for_every_agent"]);
    assert_eq!(
        offered("HELPER:"),
        ["for_child_agents", "for_every_agent", "for_the_helper"]
    );
}

#[tokio::test]
async fn a_plugin_tool_with_a_core_or_task_tool_name_an_invalid_name_or_an_activity_argument_is_refused()
 {
    let harness = harness().await;
    // The point and the code of the refusal of the plugin's tools, the
    // first and `tool`.
    let add = |tool: PluginTool| {
        add_plugin(
            &harness.runtime,
            PLUGIN,
            Contributions::new()
                .add::<Tools>(named("echo_first", AgentSelector::Every))
                .add::<Tools>(tool),
        )
        .err()
        .map(|refusal| (refusal.point, refusal.code))
    };
    let duplicate = Some(("tools", ID_TAKEN));
    let invalid = Some(("tools", INVALID_NAME));
    let too_long = "e".repeat(65);
    let takes_activity = json!({
        "type": "object",
        "properties": {
            "activity": { "type": "string" },
            "text": { "type": "string" },
        },
    });

    assert_eq!(add(named("read_file", AgentSelector::Every)), duplicate);
    for task_tool in ["start_task", "follow_up_task", "wait_for_tasks"] {
        assert_eq!(
            add(named(task_tool, AgentSelector::Every)),
            duplicate,
            "{task_tool}"
        );
    }
    assert_eq!(add(named("echo_first", AgentSelector::Every)), duplicate);
    for name in ["Echo-Text", "", "echo text", "2echo", &too_long] {
        assert_eq!(add(named(name, AgentSelector::Every)), invalid, "{name}");
    }
    assert_eq!(
        add(with_schema(
            "echo_label",
            takes_activity,
            AgentSelector::Every
        )),
        Some(("tools", RESERVED_ARGUMENT))
    );
    // The refused sets left nothing behind: the plugin starts with a valid
    // set afterwards, whose tool joins the run once, beside the core's.
    add_plugin(&harness.runtime, PLUGIN, contributions()).unwrap();
    harness.model.push_reply(reply("Hello."));
    let (conversation, run) = start(&harness, "OWNER: hello").await;
    harness.wait_for_run(&conversation, &run).await;
    let names = tool_names(&harness.model.requests_for("OWNER:")[0]);
    assert!(names.contains(&ECHO.to_owned()), "{names:?}");
    assert!(!names.contains(&"echo_first".to_owned()), "{names:?}");
    for name in ["read_file", "start_task"] {
        assert_eq!(
            names.iter().filter(|known| *known == name).count(),
            1,
            "{names:?}"
        );
    }
}

#[tokio::test]
async fn a_plugin_tool_with_the_name_of_an_executors_tool_is_refused_and_a_run_offers_each_name_once()
 {
    let harness = harness().await;
    let with_executor = || Contributions::new().add::<Executors>(executor());
    let give_task = || named(FAKE_EXECUTOR_TOOL, AgentSelector::Conversations);
    // The point and the code of a refusal.
    let refused = |plugin: &str, contributions: Contributions| {
        add_plugin(&harness.runtime, plugin, contributions)
            .err()
            .map(|refusal| (refusal.point, refusal.code))
    };

    add_plugin(&harness.runtime, FAKE_EXECUTOR_KIND, with_executor()).unwrap();
    let beside_the_executor = refused(PLUGIN, Contributions::new().add::<Tools>(give_task()));
    remove_plugin(&harness.runtime, FAKE_EXECUTOR_KIND);
    let with_its_own_executor = refused(
        FAKE_EXECUTOR_KIND,
        with_executor().add::<Tools>(give_task()),
    );
    // A tool that is on before the executor starts keeps its name, and the
    // runs leave out the executor's tool and its section.
    add_plugin(
        &harness.runtime,
        PLUGIN,
        Contributions::new().add::<Tools>(give_task()),
    )
    .unwrap();
    add_plugin(&harness.runtime, FAKE_EXECUTOR_KIND, with_executor()).unwrap();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        FAKE_EXECUTOR_TOOL,
        &json!({ "activity": "Giving" }).to_string(),
    )]));
    harness.model.push_reply(reply("Given."));
    let (conversation, run) = start(&harness, "OWNER: give it").await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let duplicate = Some(("tools", ID_TAKEN));
    assert_eq!(beside_the_executor, duplicate);
    assert_eq!(with_its_own_executor, duplicate);
    let requests = harness.model.requests_for("OWNER:");
    let names = tool_names(&requests[0]);
    assert_eq!(
        names
            .iter()
            .filter(|name| *name == FAKE_EXECUTOR_TOOL)
            .count(),
        1,
        "{names:?}"
    );
    assert!(!requests[0].instructions.contains(FAKE_EXECUTOR_SECTION));
    // The call reached the plugin's tool, which answers with its name.
    assert_eq!(result(&requests[1], "call_1"), FAKE_EXECUTOR_TOOL);
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(
        call.result.unwrap().outcome,
        ToolOutcome::Plugin {
            plugin: PLUGIN.to_owned(),
            data: None,
            failure: None,
        }
    );
    let task_calls = transcript.task_calls(0);
    assert!(task_calls.is_empty(), "{task_calls:?}");
}

#[tokio::test]
async fn a_plugin_tools_refusal_of_its_arguments_keeps_the_call_from_running() {
    let harness = harness().await;
    let refusing = Arc::new(Refusing::default());
    add_plugin(
        &harness.runtime,
        PLUGIN,
        Contributions::new().add::<Tools>(for_conversations(Arc::<Refusing>::clone(&refusing))),
    )
    .unwrap();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        ECHO,
        &json!({ "activity": "Echoing", "text": "" }).to_string(),
    )]));
    harness.model.push_reply(reply("The text was empty."));

    let (conversation, run) = start(&harness, "OWNER: echo nothing").await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let requests = harness.model.requests_for("OWNER:");
    assert_eq!(
        result(&requests[1], "call_1"),
        texts::invalid_arguments("the text is empty")
    );
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(
        call.not_run,
        Some(NotRunReason::InvalidArguments {
            detail: "the text is empty".to_owned()
        })
    );
    assert_eq!(call.result, None);
    assert!(!refusing.executed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn stop_drops_a_plugin_tools_execution_that_never_resolves() {
    let harness = harness().await;
    let endless = Arc::new(Endless::default());
    add_plugin(
        &harness.runtime,
        PLUGIN,
        Contributions::new().add::<Tools>(for_conversations(Arc::<Endless>::clone(&endless))),
    )
    .unwrap();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        ENDLESS,
        &json!({ "activity": "Waiting" }).to_string(),
    )]));

    let (conversation, run) = start(&harness, "OWNER: wait").await;
    harness
        .wait_until(&conversation, |transcript| {
            transcript
                .tool_calls(0)
                .first()
                .is_some_and(|call| call.status == ToolCallStatus::Running)
        })
        .await;
    let started = Arc::clone(&endless.started);
    tokio::time::timeout(Duration::from_secs(20), async move {
        while !started.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the execution starts");
    assert!(!endless.dropped.load(Ordering::SeqCst));
    harness.runtime.stop_reply(&conversation).await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    assert!(endless.dropped.load(Ordering::SeqCst));
    assert_eq!(transcript.run_state(&run), Some(RunState::Stopped));
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    assert_eq!(call.status, ToolCallStatus::Canceled);
    assert_eq!(call.result, None);
}

#[tokio::test]
async fn a_plugin_tools_output_is_cut_to_the_result_limit() {
    // A profile whose context budget gives its runs a result limit below
    // the most that a result may hold, so the cut shows the run's limit.
    let (context_window, output_limit) = (72_000, 4_096);
    let harness = Harness::with_options(HarnessOptions {
        use_tools: true,
        context_window,
        output_limit,
        ..HarnessOptions::default()
    })
    .await;
    // A quarter of the budget, at five bytes for every two tokens.
    let budget = usize::try_from(context_budget(context_window, output_limit)).unwrap();
    let limit = result_limit(budget * 5 / 2);
    assert!(
        MIN_RESULT_LIMIT_BYTES < limit && limit < RESULT_LIMIT_BYTES,
        "{limit}"
    );
    // One byte, then two-byte characters: a cut at most byte counts falls
    // between the bytes of a character unless it moves to one.
    let whole = format!("x{}", "é".repeat(RESULT_LIMIT_BYTES));
    add_plugin(
        &harness.runtime,
        PLUGIN,
        Contributions::new().add::<Tools>(for_conversations(Arc::new(Answering {
            name: ECHO.to_owned(),
            parameters: text_argument(),
            output: whole.clone(),
        }))),
    )
    .unwrap();
    harness.model.push_reply(tool_calls(&[(
        "call_1",
        ECHO,
        &json!({ "activity": "Echoing", "text": "a lot" }).to_string(),
    )]));
    harness.model.push_reply(reply("That was long."));

    let (conversation, run) = start(&harness, "OWNER: echo a lot").await;
    let transcript = harness.wait_for_run(&conversation, &run).await;

    let read = result(&harness.model.requests_for("OWNER:")[1], "call_1");
    assert!(read.len() <= limit, "{} of {limit}", read.len());
    let note = format!("\n{}", texts::plugin_output_cut(whole.len()));
    let head = read
        .strip_suffix(&note)
        .expect("the output ends with the note");
    assert!(head.starts_with("xé"), "{head}");
    assert!(whole.starts_with(head));
    // The head fills the room before the note, but for the byte of a
    // character that did not fit.
    let room = limit - note.len();
    assert!(
        head.len() == room || head.len() + 1 == room,
        "{} of {room}",
        head.len()
    );
    let [call] = transcript.tool_calls(0).try_into().unwrap();
    let stored = call.result.unwrap();
    assert_eq!(stored.output, read);
    assert_eq!(
        stored.outcome,
        ToolOutcome::Plugin {
            plugin: PLUGIN.to_owned(),
            data: None,
            failure: None,
        }
    );
}
