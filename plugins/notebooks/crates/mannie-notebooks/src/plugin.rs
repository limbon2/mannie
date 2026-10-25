//! The Notebooks plugin's definition (plugins/notebooks/docs/README.md): its
//! interface, its tables, its start and stop, its context provider, its
//! mention provider, its tools and the handlers of its commands and channel.
//! Its manifest, `plugins/notebooks/plugin.toml`, gives the services that its
//! start receives. It is off by default, like every plugin (D28).
//!
//! A start fails the sources that were being read when the plugin stopped,
//! follows the embedding model, contributes the providers and the tools and
//! starts the indexer, which resumes the sources that were being indexed.
//! Turning the plugin off removes the contributions, the host ends the calls
//! and the channel, and the stop ends the indexer, whose work the next start
//! resumes.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use futures_util::{StreamExt as _, stream};
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::StartContext;
use mannie_plugin_sdk::contracts::WorkItem;
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::ipc::{Handlers, Interface, PluginStream};
use mannie_plugin_sdk::plugin::{Instance, Plugin, Started};
use mannie_plugin_sdk::points::context::{ContextProviders, PluginContextProvider};
use mannie_plugin_sdk::points::mentions::{MentionProvider, Mentions};
use mannie_plugin_sdk::points::tools::Tools;
use mannie_plugin_sdk::services::file_dialog::FileDialog;
use mannie_plugin_sdk::services::models::Models;
use mannie_plugin_sdk::services::store::{Store, StoreModule};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::commands::{
    AddTextSource, AddTextSourceCommand, AddUploadPart, AddUploadPartCommand, CancelUploadCommand,
    CreateNotebook, CreateNotebookCommand, DeleteNotebookCommand, FinishUploadCommand,
    FollowNotebooksChannel, ListSearchProfilesCommand, LocatePassage, LocatePassageCommand,
    NotebookList, NotebookRef, NotebooksError, PickSources, PickSourcesCommand, Preferences,
    ReadPreferencesCommand, ReadSource, ReadSourceCommand, RemoveSourceCommand, RenameNotebook,
    RenameNotebookCommand, RetrySourceCommand, SavePreferencesCommand, SetSourceIncluded,
    SetSourceIncludedCommand, SourceRef, SourceRefusal, StartUpload, StartUploadCommand, UploadRef,
    interface,
};
use crate::indexer;
use crate::library::{Library, Timing};
use crate::mentions::NotebookMentions;
use crate::provider::NotebookContext;
use crate::store::{MODULE, NotebookStore};
use crate::tools::tools;

/// The plugin's ID, which its context provider shares.
pub const NOTEBOOKS: &str = "notebooks";

/// The code of a start that could not read the notebooks.
pub const STORAGE_FAILED: &str = "storage_failed";

/// The code of the work of sources that are being read or indexed, which the
/// UI words.
pub const INDEXING_SOURCES: &str = "indexing_sources";

/// The Notebooks plugin.
#[derive(Debug, Clone, Copy, Default)]
pub struct NotebooksPlugin;

impl NotebooksPlugin {
    /// The plugin, off.
    pub fn new() -> Self {
        Self
    }
}

/// The notebooks as they are, then after each change, until the start stops.
fn notebook_list(
    library: Arc<Library>,
    revisions: watch::Receiver<u64>,
) -> PluginStream<NotebookList> {
    stream::unfold(
        (library, revisions, true),
        |(library, mut revisions, first)| async move {
            if !first && revisions.changed().await.is_err() {
                return None;
            }
            revisions.borrow_and_update();
            let list = library.list();
            Some((list, (library, revisions, false)))
        },
    )
    .boxed()
}

/// Adds a part of a file that the owner dropped. Its bytes come from the
/// webview as base64, and the whole file is checked like any file's when it
/// finishes.
fn add_part(library: &Library, input: &AddUploadPart) -> Result<(), NotebooksError> {
    let part = BASE64.decode(input.data.as_bytes()).map_err(|_invalid| {
        // A part that cannot be decoded ruins the file, so the upload goes.
        library.cancel_upload(&input.upload_id);
        NotebooksError::Refused {
            reason: SourceRefusal::Unreadable,
        }
    })?;
    library.add_upload_part(&input.upload_id, input.index, &part)
}

/// The handlers of a start, which reach its library.
fn handlers(library: &Arc<Library>) -> Handlers {
    let each = || Arc::clone(library);
    let (creating, renaming, deleting, picking, writing) = (each(), each(), each(), each(), each());
    let (starting, adding, finishing, cancelling) = (each(), each(), each(), each());
    let (removing, retrying, switching, reading, locating) =
        (each(), each(), each(), each(), each());
    let (preferring, saving, listing, following) = (each(), each(), each(), each());
    Handlers::new()
        .command::<CreateNotebookCommand, _, _>(move |input: CreateNotebook| {
            let library = Arc::clone(&creating);
            async move { library.create_notebook(&input.name).await }
        })
        .command::<RenameNotebookCommand, _, _>(move |input: RenameNotebook| {
            let library = Arc::clone(&renaming);
            async move {
                library
                    .rename_notebook(&input.notebook_id, &input.name)
                    .await
            }
        })
        .command::<DeleteNotebookCommand, _, _>(move |input: NotebookRef| {
            let library = Arc::clone(&deleting);
            async move { library.delete_notebook(&input.notebook_id).await }
        })
        .command::<PickSourcesCommand, _, _>(move |input: PickSources| {
            let library = Arc::clone(&picking);
            async move {
                library
                    .pick_sources(&input.notebook_id, &input.filter_name)
                    .await
            }
        })
        .command::<StartUploadCommand, _, _>(move |input: StartUpload| {
            std::future::ready(starting.start_upload(&input.notebook_id, &input.name))
        })
        .command::<AddUploadPartCommand, _, _>(move |input: AddUploadPart| {
            std::future::ready(add_part(&adding, &input))
        })
        .command::<FinishUploadCommand, _, _>(move |input: UploadRef| {
            let library = Arc::clone(&finishing);
            async move { library.finish_upload(&input.upload_id).await }
        })
        .command::<CancelUploadCommand, _, _>(move |input: UploadRef| {
            cancelling.cancel_upload(&input.upload_id);
            std::future::ready(Ok::<_, NotebooksError>(()))
        })
        .command::<AddTextSourceCommand, _, _>(move |input: AddTextSource| {
            let library = Arc::clone(&writing);
            async move {
                library
                    .add_text(&input.notebook_id, &input.title, &input.text)
                    .await
            }
        })
        .command::<RemoveSourceCommand, _, _>(move |input: SourceRef| {
            let library = Arc::clone(&removing);
            async move { library.remove_source(&input.source_id).await }
        })
        .command::<RetrySourceCommand, _, _>(move |input: SourceRef| {
            let library = Arc::clone(&retrying);
            async move { library.retry_source(&input.source_id).await }
        })
        .command::<SetSourceIncludedCommand, _, _>(move |input: SetSourceIncluded| {
            let library = Arc::clone(&switching);
            async move { library.set_included(&input.source_id, input.included).await }
        })
        .command::<ReadSourceCommand, _, _>(move |input: ReadSource| {
            let library = Arc::clone(&reading);
            async move {
                library
                    .read_source(&input.source_id, input.first_section)
                    .await
            }
        })
        .command::<LocatePassageCommand, _, _>(move |input: LocatePassage| {
            let library = Arc::clone(&locating);
            async move { library.locate(&input.key).await }
        })
        .command::<ReadPreferencesCommand, _, _>(move |()| {
            std::future::ready(Ok::<_, NotebooksError>(preferring.preferences()))
        })
        .command::<SavePreferencesCommand, _, _>(move |input: Preferences| {
            let library = Arc::clone(&saving);
            async move { library.save_preferences(input).await }
        })
        .command::<ListSearchProfilesCommand, _, _>(move |()| {
            let library = Arc::clone(&listing);
            async move { library.search_profiles().await }
        })
        .channel::<FollowNotebooksChannel, _, _>(move |()| {
            let revisions = following.follow();
            std::future::ready(Ok::<_, NotebooksError>(notebook_list(
                Arc::clone(&following),
                revisions,
            )))
        })
}

/// Follows the changes of the owner's search profiles, so that a new model of
/// the embedding profile indexes the sources again.
fn follow_profiles(library: Arc<Library>) -> JoinHandle<()> {
    let mut changes = library.models.changes();
    tokio::spawn(async move {
        while let Some(profile) = changes.next().await {
            tracing::debug!(profile, "a search profile changed");
            if let Err(error) = library.follow_models().await {
                tracing::warn!(error.code = %error, "the notebooks could not follow a change of the search profiles");
            }
        }
    })
}

impl NotebooksPlugin {
    async fn start_now(context: StartContext) -> Result<Started, StartFailure> {
        let store = NotebookStore::new(context.service::<Store>()?);
        let dialog = context.service::<FileDialog>()?;
        let models = context.service::<Models>()?;
        let timing = Timing::for_host(context.host().profile);
        let library = Library::load(store, models, dialog, context.clock(), timing)
            .await
            .map_err(|error| {
                tracing::error!(error.code = %error, "the Notebooks plugin could not read its notebooks");
                StartFailure::new(STORAGE_FAILED)
            })?;
        let provider = PluginContextProvider::new(
            NOTEBOOKS,
            Arc::new(NotebookContext::new(Arc::clone(&library))),
        );
        let mentions: Arc<dyn MentionProvider> =
            Arc::new(NotebookMentions::new(Arc::clone(&library)));
        let contributions = tools(&library).into_iter().fold(
            Contributions::new()
                .add::<ContextProviders>(provider)
                .add::<Mentions>(mentions),
            Contributions::add::<Tools>,
        );
        let handlers = handlers(&library);
        let tasks = vec![
            tokio::spawn(indexer::run(Arc::clone(&library))),
            follow_profiles(Arc::clone(&library)),
        ];
        tracing::info!("the Notebooks plugin started");
        Ok(Started::new(NotebooksInstance { library, tasks })
            .with_contributions(contributions)
            .with_handlers(handlers))
    }
}

impl Plugin for NotebooksPlugin {
    fn interface(&self) -> Interface {
        interface()
    }

    fn store_module(&self) -> Option<StoreModule> {
        Some(MODULE)
    }

    fn start(&self, context: StartContext) -> BoxFuture<'static, Result<Started, StartFailure>> {
        Box::pin(Self::start_now(context))
    }
}

/// The plugin while it is on: its library, its indexer and its follower of
/// the search profiles.
#[derive(Debug)]
struct NotebooksInstance {
    library: Arc<Library>,
    tasks: Vec<JoinHandle<()>>,
}

impl Instance for NotebooksInstance {
    fn work(&self) -> BoxFuture<'static, Vec<WorkItem>> {
        let indexing = self.library.indexing_count();
        let work = if indexing > 0 {
            vec![WorkItem::new(INDEXING_SOURCES, indexing)]
        } else {
            Vec::new()
        };
        Box::pin(std::future::ready(work))
    }

    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        // The host removed the provider and ended the calls and the channel
        // first. A source that was being indexed resumes with the next
        // start; one that was being read fails then, since its bytes go.
        for task in &self.tasks {
            task.abort();
        }
        drop(self.library);
        tracing::info!("the Notebooks plugin stopped");
        Box::pin(std::future::ready(()))
    }
}
