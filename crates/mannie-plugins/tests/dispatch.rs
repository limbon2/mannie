//! The dispatch of plugin calls (ADR-0034, decision 8) without a webview, on
//! a temporary store: the host's refusals in their order, the plugin's
//! answers and errors, a handler that panics, channels and the host's ends,
//! and turning a plugin off during a call that ends in time and during one
//! that never resolves.

#![cfg(test)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_util::{StreamExt, stream};
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::{HostInfo, HostProfile, StartContext};
use mannie_plugin_sdk::contracts::{Clock, SubscriptionEnd, UtcMillis};
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::ipc::{
    ChannelMessage, ChannelSpec, CommandSpec, ErrorValue, Handlers, HostRefusal, InputRefusal,
    Interface, PluginError, PluginStream, Validate,
};
use mannie_plugin_sdk::plugin::{Instance, Plugin, PluginEntry, Started};
use mannie_plugins::{CallFailure, PluginHost, PluginHostDeps, PluginList};
use mannie_store::{Store, StoreOptions};
use mannie_types::clock::ManualClock;
use mannie_types::plugins::PluginState;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::Notify;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the plugin, its handlers and its instance did, in order.
#[derive(Debug, Default)]
struct Record {
    events: Mutex<Vec<String>>,
    fail_start: AtomicBool,
    /// Holds the next start until a test lets it go.
    start_gate: Mutex<Option<Arc<Notify>>>,
}

impl Record {
    fn log(&self, event: &str) {
        lock(&self.events).push(event.to_owned());
    }

    fn events(&self) -> Vec<String> {
        lock(&self.events).clone()
    }
}

/// Logs that a handler's future was dropped, as an abort drops it.
struct DropLog {
    record: Arc<Record>,
    event: &'static str,
}

impl Drop for DropLog {
    fn drop(&mut self) {
        self.record.log(self.event);
    }
}

#[derive(Debug, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AddNote {
    text: String,
}

impl Validate for AddNote {
    fn validate(&self) -> Result<(), InputRefusal> {
        if self.text.is_empty() {
            return Err(InputRefusal::new("text"));
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
struct NoteView {
    text: String,
}

#[derive(Debug, Serialize, specta::Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
enum NotesError {
    #[error("a note has at most {max_characters} characters")]
    TooLong { max_characters: u32 },
}

impl PluginError for NotesError {
    fn code(&self) -> &'static str {
        "notes.too_long"
    }
}

const MAX_CHARACTERS: usize = 20;

enum AddNoteCommand {}

impl CommandSpec for AddNoteCommand {
    const NAME: &'static str = "add_note";
    const DOCS: &'static str = "Adds a note.";
    type Input = AddNote;
    type Output = NoteView;
    type Error = NotesError;
}

/// A command whose input may have at most 16 bytes as JSON.
enum SmallCommand {}

impl CommandSpec for SmallCommand {
    const NAME: &'static str = "small";
    const DOCS: &'static str = "Takes a small note.";
    const MAX_INPUT_BYTES: u32 = 16;
    type Input = AddNote;
    type Output = ();
    type Error = NotesError;
}

/// A command whose handler never resolves.
enum HangCommand {}

impl CommandSpec for HangCommand {
    const NAME: &'static str = "hang";
    const DOCS: &'static str = "Never answers.";
    type Input = ();
    type Output = ();
    type Error = NotesError;
}

/// A command whose handler answers after 100 milliseconds.
enum SlowCommand {}

impl CommandSpec for SlowCommand {
    const NAME: &'static str = "slow";
    const DOCS: &'static str = "Answers after a while.";
    type Input = ();
    type Output = u32;
    type Error = NotesError;
}

/// A command whose handler panics.
enum PanicCommand {}

impl CommandSpec for PanicCommand {
    const NAME: &'static str = "panic_now";
    const DOCS: &'static str = "Panics.";
    type Input = ();
    type Output = ();
    type Error = NotesError;
}

/// A channel with two items, which then stays open.
enum FollowChannel {}

impl ChannelSpec for FollowChannel {
    const NAME: &'static str = "follow_notes";
    const DOCS: &'static str = "Follows the notes.";
    type Input = ();
    type Item = u32;
    type Error = NotesError;
}

/// A channel with one item, whose stream then ends.
enum FiniteChannel {}

impl ChannelSpec for FiniteChannel {
    const NAME: &'static str = "finite";
    const DOCS: &'static str = "Sends one note and ends.";
    type Input = ();
    type Item = u32;
    type Error = NotesError;
}

#[derive(Debug, Default)]
struct Notes {
    record: Arc<Record>,
}

fn handlers(record: &Arc<Record>) -> Handlers {
    let hang = Arc::clone(record);
    let slow = Arc::clone(record);
    Handlers::new()
        .command::<AddNoteCommand, _, _>(|input: AddNote| async move {
            if input.text.chars().count() > MAX_CHARACTERS {
                return Err(NotesError::TooLong { max_characters: 20 });
            }
            Ok(NoteView { text: input.text })
        })
        .command::<SmallCommand, _, _>(|_input: AddNote| async { Ok(()) })
        .command::<HangCommand, _, _>(move |()| {
            let record = Arc::clone(&hang);
            async move {
                record.log("hang started");
                let _dropped = DropLog {
                    record: Arc::clone(&record),
                    event: "hang dropped",
                };
                std::future::pending::<()>().await;
                Ok(())
            }
        })
        .command::<SlowCommand, _, _>(move |()| {
            let record = Arc::clone(&slow);
            async move {
                record.log("slow started");
                tokio::time::sleep(Duration::from_millis(100)).await;
                record.log("slow ended");
                Ok(7)
            }
        })
        .command::<PanicCommand, _, _>(|()| async {
            panic!("the test's handler panics");
        })
        .channel::<FollowChannel, _, _>(|()| async {
            let items: PluginStream<u32> = stream::iter([1, 2]).chain(stream::pending()).boxed();
            Ok(items)
        })
        .channel::<FiniteChannel, _, _>(|()| async {
            let items: PluginStream<u32> = stream::iter([1]).boxed();
            Ok(items)
        })
}

impl Plugin for Notes {
    fn interface(&self) -> Interface {
        Interface::new()
            .command::<AddNoteCommand>()
            .command::<SmallCommand>()
            .command::<HangCommand>()
            .command::<SlowCommand>()
            .command::<PanicCommand>()
            .channel::<FollowChannel>()
            .channel::<FiniteChannel>()
    }

    fn start(&self, _context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>> {
        let record = Arc::clone(&self.record);
        Box::pin(async move {
            let gate = lock(&record.start_gate).take();
            if let Some(gate) = gate {
                gate.notified().await;
            }
            if record.fail_start.load(Ordering::SeqCst) {
                return Err(StartFailure::new("storage_failed"));
            }
            record.log("started");
            let instance = NotesInstance {
                record: Arc::clone(&record),
            };
            Ok(Started::new(instance).with_handlers(handlers(&record)))
        })
    }
}

#[derive(Debug)]
struct NotesInstance {
    record: Arc<Record>,
}

impl Instance for NotesInstance {
    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        self.record.log("stopped");
        Box::pin(std::future::ready(()))
    }
}

const MANIFEST: &str = "# The test's plugin.\nid = \"notes\"\nsdk = 1\norder = 10\n\n[rust]\nentry = \"mannie-notes\"\n";

/// A host of the plugin `notes` on a temporary store.
struct Fixture {
    _directory: TempDir,
    record: Arc<Record>,
    host: PluginHost,
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let plugin = Notes::default();
        let record = Arc::clone(&plugin.record);
        let hosted: Arc<dyn Plugin> = Arc::new(plugin);
        let list = PluginList::new(vec![PluginEntry::new(MANIFEST, hosted)]).unwrap();
        let clock: Arc<dyn Clock> = Arc::new(ManualClock::new(UtcMillis::from_millis(1_234)));
        let options = StoreOptions::new(directory.path().join("backups"), Arc::clone(&clock))
            .with_plugin_modules(list.store_modules());
        let store = Store::open(&directory.path().join("mannie.sqlite"), &options).unwrap();
        let host = PluginHost::new(PluginHostDeps {
            store: store.clone(),
            plugins: list,
            providers: mannie_store::service_providers(&store),
            consumers: Vec::new(),
            clock,
            host: HostInfo::new("0.3.0", HostProfile::Validation),
        })
        .unwrap();
        host.start_chosen().await;
        Self {
            _directory: directory,
            record,
            host,
        }
    }

    /// A host whose plugin is on.
    async fn on() -> Self {
        let fixture = Self::new().await;
        fixture.host.set_enabled("notes", true).await.unwrap();
        fixture
    }

    async fn invoke(&self, command: &str, input: Value) -> Result<Value, CallFailure> {
        self.host.invoke("notes", command, input).await
    }

    fn state(&self) -> PluginState {
        self.host.views()[0].state
    }
}

fn refused(refusal: HostRefusal) -> Result<Value, CallFailure> {
    Err(CallFailure::Host(refusal))
}

fn plugin_off() -> Result<Value, CallFailure> {
    refused(HostRefusal::plugin_off("notes"))
}

/// Waits until `done` holds, letting other tasks run.
async fn until(done: impl Fn() -> bool) {
    // A yield count can expire before the store's blocking work runs.
    tokio::time::timeout(Duration::from_secs(10), async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the condition never held");
}

#[tokio::test]
async fn a_command_answers_with_its_output_and_the_plugin_s_error_crosses_as_its_own() {
    let fixture = Fixture::on().await;

    let added = fixture.invoke("add_note", json!({ "text": "milk" })).await;
    let refused = fixture
        .invoke("add_note", json!({ "text": "x".repeat(21) }))
        .await;

    assert_eq!(added, Ok(json!({ "text": "milk" })));
    assert_eq!(
        refused,
        Err(CallFailure::Plugin(ErrorValue::new(
            "notes.too_long",
            json!({ "code": "too_long", "maxCharacters": 20 }),
            "a note has at most 20 characters",
        )))
    );
}

#[tokio::test]
async fn every_state_but_on_refuses_with_plugin_off_before_the_input_is_read() {
    let fixture = Fixture::new().await;
    // An input that is too large and does not decode: the plugin's state
    // decides first.
    let large = json!("x".repeat(100_000));

    let off = fixture.invoke("add_note", large.clone()).await;
    fixture.record.fail_start.store(true, Ordering::SeqCst);
    fixture.host.set_enabled("notes", true).await.unwrap();
    let failed_state = fixture.state();
    let failed = fixture.invoke("add_note", large.clone()).await;

    fixture.record.fail_start.store(false, Ordering::SeqCst);
    let gate = Arc::new(Notify::new());
    *lock(&fixture.record.start_gate) = Some(Arc::clone(&gate));
    let starting_host = fixture.host.clone();
    let starting = tokio::spawn(async move { starting_host.set_enabled("notes", true).await });
    until(|| fixture.state() == PluginState::Starting).await;
    let while_starting = fixture.invoke("add_note", large.clone()).await;
    gate.notify_one();
    starting.await.unwrap().unwrap();
    let on = fixture.invoke("add_note", json!({ "text": "milk" })).await;

    fixture.host.set_enabled("notes", false).await.unwrap();
    let turned_off = fixture.invoke("add_note", large).await;

    assert_eq!(off, plugin_off());
    assert_eq!(failed_state, PluginState::Failed);
    assert_eq!(failed, plugin_off());
    assert_eq!(while_starting, plugin_off());
    assert_eq!(on, Ok(json!({ "text": "milk" })));
    assert_eq!(turned_off, plugin_off());
}

#[tokio::test]
async fn quitting_refuses_every_call_with_shutting_down() {
    let fixture = Fixture::on().await;

    fixture.host.shutdown().await;
    let answer = fixture.invoke("add_note", json!({ "text": "milk" })).await;
    let channel = fixture
        .host
        .open_channel("notes", "follow_notes", Value::Null)
        .await
        .err();

    assert_eq!(answer, refused(HostRefusal::ShuttingDown));
    assert_eq!(channel, Some(CallFailure::Host(HostRefusal::ShuttingDown)));
}

#[tokio::test]
async fn a_command_or_channel_that_the_plugin_does_not_have_is_unknown() {
    let fixture = Fixture::on().await;

    let command = fixture.invoke("delete_everything", Value::Null).await;
    let channel = fixture
        .host
        .open_channel("notes", "add_note", json!({ "text": "milk" }))
        .await
        .err();
    let as_command = fixture.invoke("follow_notes", Value::Null).await;

    assert_eq!(command, refused(HostRefusal::UnknownCommand));
    assert_eq!(
        channel,
        Some(CallFailure::Host(HostRefusal::UnknownCommand))
    );
    assert_eq!(as_command, refused(HostRefusal::UnknownCommand));
}

#[tokio::test]
async fn an_input_larger_than_the_spec_allows_is_refused_before_it_is_decoded() {
    let fixture = Fixture::on().await;

    let fits = fixture.invoke("small", json!({ "text": "milk" })).await;
    let too_large = fixture
        .invoke("small", json!({ "text": "milk and honey" }))
        .await;
    let undecodable = fixture
        .invoke("small", json!([1, 2, 3, 4, 5, 6, 7, 8, 9]))
        .await;
    let small_but_undecodable = fixture.invoke("small", json!([1])).await;

    assert_eq!(fits, Ok(Value::Null));
    assert_eq!(
        too_large,
        refused(HostRefusal::InputTooLarge { max_bytes: 16 })
    );
    assert_eq!(
        undecodable,
        refused(HostRefusal::InputTooLarge { max_bytes: 16 })
    );
    assert_eq!(
        small_but_undecodable,
        refused(HostRefusal::InvalidInput { field: None })
    );
}

#[tokio::test]
async fn an_input_that_does_not_decode_or_breaks_a_rule_never_reaches_the_handler() {
    let fixture = Fixture::on().await;

    let unknown_field = fixture
        .invoke("add_note", json!({ "text": "milk", "urgent": true }))
        .await;
    let missing_field = fixture.invoke("add_note", json!({})).await;
    let wrong_type = fixture.invoke("add_note", json!({ "text": 7 })).await;
    let broken_rule = fixture.invoke("add_note", json!({ "text": "" })).await;
    let unit_with_fields = fixture.invoke("slow", json!({ "now": true })).await;

    for answer in [unknown_field, missing_field, wrong_type, unit_with_fields] {
        assert_eq!(answer, refused(HostRefusal::InvalidInput { field: None }));
    }
    assert_eq!(
        broken_rule,
        refused(HostRefusal::InvalidInput {
            field: Some("text".to_owned())
        })
    );
    assert_eq!(fixture.record.events(), ["started"]);
}

#[tokio::test]
async fn every_command_refuses_an_unknown_field() {
    let fixture = Fixture::on().await;
    let interface = Notes::default().interface();

    for command in interface.command_names() {
        // Each input is an object or nothing, and no input has this field.
        // The probe is small enough for every command's limit.
        let probe = json!({ "probe": 1 });
        let answer = fixture.invoke(command, probe).await;
        assert_eq!(
            answer,
            refused(HostRefusal::InvalidInput { field: None }),
            "{command}"
        );
    }
}

#[tokio::test]
async fn a_handler_that_panics_is_answered_with_host_service_failed_and_the_plugin_keeps_working() {
    let fixture = Fixture::on().await;

    let panicked = fixture.invoke("panic_now", Value::Null).await;
    let after = fixture.invoke("add_note", json!({ "text": "milk" })).await;

    assert_eq!(panicked, refused(HostRefusal::HostServiceFailed));
    assert_eq!(after, Ok(json!({ "text": "milk" })));
    assert_eq!(fixture.state(), PluginState::On);
}

#[tokio::test]
async fn a_channel_carries_the_plugin_s_items_and_ends_with_plugin_off_when_the_plugin_stops() {
    let fixture = Fixture::on().await;
    let mut channel = fixture
        .host
        .open_channel("notes", "follow_notes", Value::Null)
        .await
        .unwrap();

    let first = channel.next().await;
    let second = channel.next().await;
    fixture.host.set_enabled("notes", false).await.unwrap();
    let end = channel.next().await;
    let after = channel.next().await;

    assert_eq!(first, Some(ChannelMessage::Item { item: json!(1) }));
    assert_eq!(second, Some(ChannelMessage::Item { item: json!(2) }));
    assert_eq!(
        end,
        Some(ChannelMessage::Ended {
            reason: SubscriptionEnd::PluginOff
        })
    );
    assert_eq!(after, None);
}

#[tokio::test]
async fn a_channel_ends_with_shutdown_when_mannie_quits() {
    let fixture = Fixture::on().await;
    let channel = fixture
        .host
        .open_channel("notes", "follow_notes", Value::Null)
        .await
        .unwrap();

    fixture.host.shutdown().await;
    let messages: Vec<ChannelMessage<Value>> = channel.collect().await;

    assert_eq!(
        messages.last(),
        Some(&ChannelMessage::Ended {
            reason: SubscriptionEnd::Shutdown
        })
    );
}

#[tokio::test]
async fn a_stream_that_ends_by_itself_ends_the_channel_with_lagged() {
    let fixture = Fixture::on().await;
    let channel = fixture
        .host
        .open_channel("notes", "finite", Value::Null)
        .await
        .unwrap();

    let messages: Vec<ChannelMessage<Value>> = channel.collect().await;

    assert_eq!(
        messages,
        [
            ChannelMessage::Item { item: json!(1) },
            ChannelMessage::Ended {
                reason: SubscriptionEnd::Lagged
            },
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn turning_off_during_a_handler_that_never_resolves_aborts_it_and_stops_the_plugin_after_it()
{
    let fixture = Fixture::on().await;
    let host = fixture.host.clone();
    let call = tokio::spawn(async move { host.invoke("notes", "hang", Value::Null).await });
    until(|| fixture.record.events().contains(&"hang started".to_owned())).await;

    let started = tokio::time::Instant::now();
    let view = fixture.host.set_enabled("notes", false).await.unwrap();
    let waited = started.elapsed();
    let answer = call.await.unwrap();

    assert_eq!(answer, plugin_off());
    assert_eq!(view.state, PluginState::Off);
    assert!(waited >= Duration::from_secs(5), "{waited:?}");
    assert_eq!(
        fixture.record.events(),
        ["started", "hang started", "hang dropped", "stopped"]
    );
}

#[tokio::test(start_paused = true)]
async fn a_call_aborted_because_mannie_quits_is_answered_with_shutting_down() {
    let fixture = Fixture::on().await;
    let host = fixture.host.clone();
    let call = tokio::spawn(async move { host.invoke("notes", "hang", Value::Null).await });
    until(|| fixture.record.events().contains(&"hang started".to_owned())).await;

    fixture.host.shutdown().await;
    let answer = call.await.unwrap();

    assert_eq!(answer, refused(HostRefusal::ShuttingDown));
    assert_eq!(
        fixture.record.events(),
        ["started", "hang started", "hang dropped", "stopped"]
    );
}

#[tokio::test(start_paused = true)]
async fn turning_off_waits_for_a_call_that_ends_in_time() {
    let fixture = Fixture::on().await;
    let host = fixture.host.clone();
    let call = tokio::spawn(async move { host.invoke("notes", "slow", Value::Null).await });
    until(|| fixture.record.events().contains(&"slow started".to_owned())).await;

    fixture.host.set_enabled("notes", false).await.unwrap();
    let answer = call.await.unwrap();

    assert_eq!(answer, Ok(json!(7)));
    assert_eq!(
        fixture.record.events(),
        ["started", "slow started", "slow ended", "stopped"]
    );
}

#[tokio::test]
async fn the_plugin_s_view_lists_the_commands_and_channels_of_its_interface() {
    let fixture = Fixture::new().await;

    let uses = fixture.host.views()[0].uses.clone();

    assert_eq!(
        uses.commands,
        ["add_note", "small", "hang", "slow", "panic_now"]
    );
    assert_eq!(uses.channels, ["follow_notes", "finite"]);
}

#[tokio::test]
async fn a_plugin_turned_on_again_answers_from_its_new_start() {
    let fixture = Fixture::on().await;

    fixture.host.set_enabled("notes", false).await.unwrap();
    fixture.host.set_enabled("notes", true).await.unwrap();
    let answer = fixture.invoke("add_note", json!({ "text": "milk" })).await;

    assert_eq!(answer, Ok(json!({ "text": "milk" })));
    assert_eq!(fixture.record.events(), ["started", "stopped", "started"]);
}
