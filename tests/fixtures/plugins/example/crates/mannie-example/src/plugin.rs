//! The plugin's definition: its interface, its table, its start and stop,
//! and the handlers of its command and channel. It is off by default, like
//! every plugin.
//!
//! Each start hands the notes that `add_note` keeps to the channels that
//! follow them, which end when the plugin stops. The host answers
//! `plugin_off` while the plugin is off (ADR-0034, decision 8).

use std::sync::Arc;

use futures_util::StreamExt as _;
use futures_util::stream;
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::StartContext;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::ipc::{Handlers, Interface, PluginStream};
use mannie_plugin_sdk::plugin::{Instance, Plugin, Started};
use mannie_plugin_sdk::points::sections::{Section, SectionBody, Sections};
use mannie_plugin_sdk::services::store::{
    Durability, Expect, Migration, ModuleStoreApi, Statement, Store, StoreModule, WriteBatch,
};
use tokio::sync::broadcast;

use crate::commands::{AddNote, AddNoteCommand, ExampleError, FollowNotesChannel, Note, interface};

/// The plugin's ID.
pub const EXAMPLE: &str = "example";

/// The prefix of the plugin's tables, which names its module of the store:
/// the ID with underscores in place of hyphens.
pub const PREFIX: &str = "example";

/// The notes that a start keeps for a channel that falls behind; a slower
/// channel misses the older ones.
const CHANNEL_CAPACITY: usize = 64;

const MIGRATIONS: &[Migration] = &[Migration::new(
    1,
    "create the table of the notes",
    "CREATE TABLE example_notes (id TEXT PRIMARY KEY, text TEXT NOT NULL)",
)];

const INSERT_NOTE: &str = "INSERT INTO example_notes (id, text) VALUES (?1, ?2)";

/// The plugin's module of the store: one table, `example_notes`.
pub const MODULE: StoreModule = StoreModule::new(PREFIX, MIGRATIONS, &[INSERT_NOTE]);

/// The plugin's part of the system prompt while it is on.
const SECTION: &str =
    "Notes that the owner keeps with the plugin example stay outside the conversation.";

/// The plugin.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExamplePlugin;

impl ExamplePlugin {
    /// The plugin, off.
    pub fn new() -> Self {
        Self
    }
}

/// Keeps a note in the plugin's table, then hands it to the channels that
/// follow the notes.
async fn add_note(
    store: Arc<dyn ModuleStoreApi>,
    notes: broadcast::Sender<Note>,
    input: AddNote,
) -> Result<Note, ExampleError> {
    let note = Note {
        id: uuid::Uuid::now_v7().to_string(),
        text: input.text,
    };
    let insert = Statement::new(INSERT_NOTE)
        .bind(note.id.as_str())
        .bind(note.text.as_str());
    store
        .write(
            WriteBatch::new().push(insert, Expect::Changes(1)),
            Durability::Normal,
        )
        .await
        .map_err(|error| {
            tracing::warn!(%error, "the plugin example could not store a note");
            ExampleError::StorageFailed
        })?;
    // A start without channels that follow the notes has no receiver, which
    // is not a failure.
    if notes.send(note.clone()).is_err() {
        tracing::debug!("no channel follows the notes");
    }
    Ok(note)
}

/// The notes that a start keeps from now on, until it stops.
fn follow_notes(notes: &broadcast::Sender<Note>) -> PluginStream<Note> {
    stream::unfold(notes.subscribe(), |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(note) => return Some((note, receiver)),
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::debug!(missed, "a channel of notes fell behind");
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    })
    .boxed()
}

/// The handlers of a start, which reach its store and its notes.
fn handlers(store: &Arc<dyn ModuleStoreApi>, notes: &broadcast::Sender<Note>) -> Handlers {
    let adding = Arc::clone(store);
    let added = notes.clone();
    let followed = notes.clone();
    Handlers::new()
        .command::<AddNoteCommand, _, _>(move |input| {
            add_note(Arc::clone(&adding), added.clone(), input)
        })
        .channel::<FollowNotesChannel, _, _>(move |()| {
            std::future::ready(Ok(follow_notes(&followed)))
        })
}

impl ExamplePlugin {
    /// Starts the plugin on the store that the host bound to it.
    fn start_now(context: &StartContext) -> Result<Started, StartFailure> {
        let store = context.service::<Store>()?;
        let (notes, _receiver) = broadcast::channel(CHANNEL_CAPACITY);
        let section = Section::new("example-notes", 900, SectionBody::Fixed(SECTION.to_owned()));
        let handlers = handlers(&store, &notes);
        tracing::info!("the plugin example started");
        Ok(Started::new(ExampleInstance { notes })
            .with_contributions(Contributions::new().add::<Sections>(section))
            .with_handlers(handlers))
    }
}

impl Plugin for ExamplePlugin {
    fn interface(&self) -> Interface {
        interface()
    }

    fn store_module(&self) -> Option<StoreModule> {
        Some(MODULE)
    }

    fn start(&self, context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>> {
        Box::pin(std::future::ready(Self::start_now(&context)))
    }
}

/// The plugin while it is on: the sender of its notes, whose channels end
/// when it stops.
#[derive(Debug)]
struct ExampleInstance {
    notes: broadcast::Sender<Note>,
}

impl Instance for ExampleInstance {
    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        // The host ended the plugin's calls and channels first; dropping the
        // sender ends any stream that is still open.
        drop(self.notes);
        tracing::info!("the plugin example stopped");
        Box::pin(std::future::ready(()))
    }
}
