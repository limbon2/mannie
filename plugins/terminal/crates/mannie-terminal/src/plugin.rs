//! The terminal plugin's definition (ADR-0053; plugins/terminal/docs/
//! README.md): its interface, its table, its start and stop, its
//! contributions and the handlers of its commands and channels. Its
//! manifest, `plugins/terminal/plugin.toml`, gives the services that its
//! start receives. It is off by default, like every plugin (D28).
//!
//! A start reads the settings, follows the core's hosts and contributes the
//! tools `list_terminals` and `run_in_terminal`, the section and the mention
//! provider. Turning the plugin off removes them, the host drops the calls
//! that run, whose guards interrupt their commands, and the stop ends every
//! terminal.

use std::path::PathBuf;
use std::sync::Arc;

use futures_util::{StreamExt as _, stream};
use mannie_plugin_sdk::BoxFuture;
use mannie_plugin_sdk::context::StartContext;
use mannie_plugin_sdk::contracts::{MentionEntry, WorkItem};
use mannie_plugin_sdk::contributions::Contributions;
use mannie_plugin_sdk::error::StartFailure;
use mannie_plugin_sdk::ipc::{Handlers, Interface, PluginStream};
use mannie_plugin_sdk::plugin::{Instance, Plugin, Started};
use mannie_plugin_sdk::points::mentions::{MentionProvider, Mentions};
use mannie_plugin_sdk::points::sections::{Section, SectionBody, Sections};
use mannie_plugin_sdk::points::tools::{AgentSelector, PluginTool, Tools};
use mannie_plugin_sdk::services::store::{Store, StoreModule};
use mannie_plugin_sdk::services::terminals::{
    TerminalHostList, TerminalSize, Terminals as TerminalsService,
};
use tokio::sync::{oneshot, watch};

use crate::blocks::Author;
use crate::commands::{
    AcknowledgeOutput, AcknowledgeOutputCommand, BlockRef, BlockTextCommand, BlockTextView,
    ConversationRef, EndTerminalCommand, FollowHostsChannel, FollowTerminal, FollowTerminalChannel,
    FollowTerminalsChannel, GetSettingsCommand, HostChoice as HostInput, MentionBlockCommand,
    MentionSelection, MentionSelectionCommand, OpenTerminal, OpenTerminalCommand, ResizeTerminal,
    ResizeTerminalCommand, TerminalError, TerminalRef, TerminalSettings, TerminalSummary,
    TerminalUpdate, UpdateSettingsCommand, WriteTerminal, WriteTerminalCommand, interface,
};
use crate::mentions::{CommandMentions, block_entry, selection_entry};
use crate::registry::{HostChoice, OpenRequest, Terminals};
use crate::section::{AGENTS, SECTION_ID, SECTION_POSITION, TerminalsSection};
use crate::store::{MODULE, SettingsStore};
use crate::tools::{ListTerminals, RunInTerminal};
use crate::views::{VIEW_QUEUE, View};

/// The plugin's ID.
pub const TERMINAL: &str = "terminal";

/// The code of a start that could not read the settings.
pub const STORAGE_FAILED: &str = "storage_failed";

/// The code of the plugin's work in progress: terminals that run a command.
pub const RUNNING_COMMANDS: &str = "running_commands";

/// The terminal plugin.
#[derive(Debug, Clone, Copy, Default)]
pub struct TerminalPlugin;

impl TerminalPlugin {
    /// The plugin, off.
    pub fn new() -> Self {
        Self
    }
}

/// What the handlers of a start reach.
#[derive(Debug, Clone)]
struct Shared {
    terminals: Terminals,
    settings: SettingsStore,
}

impl Shared {
    async fn open(&self, input: OpenTerminal) -> Result<TerminalSummary, TerminalError> {
        let host = match input.host {
            HostInput::ThisComputer { shell } => HostChoice::ThisComputer {
                shell: shell.map(PathBuf::from),
            },
            HostInput::Plugin { plugin, key } => HostChoice::Plugin { plugin, key },
        };
        let size =
            TerminalSize::new(input.columns, input.rows).ok_or(TerminalError::OpenFailed {
                reason: "size".to_owned(),
            })?;
        let entry = self
            .terminals
            .open(OpenRequest {
                conversation: input.conversation_id.to_string(),
                host,
                folder: input.folder.map(PathBuf::from),
                size,
                opened_by: Author::Owner,
                expect: None,
            })
            .await?;
        Ok(entry.summary())
    }

    fn entry(
        &self,
        conversation: &str,
        number: u32,
    ) -> Result<Arc<crate::registry::Entry>, TerminalError> {
        self.terminals
            .entry(conversation, number)
            .ok_or(TerminalError::TerminalNotFound)
    }

    async fn write(&self, input: WriteTerminal) -> Result<(), TerminalError> {
        let entry = self.entry(&input.conversation_id.to_string(), input.terminal)?;
        entry
            .session
            .write_owner(input.data)
            .await
            .map_err(|_ended| TerminalError::TerminalEnded)
    }

    async fn resize(&self, input: ResizeTerminal) -> Result<(), TerminalError> {
        let entry = self.entry(&input.conversation_id.to_string(), input.terminal)?;
        let size =
            TerminalSize::new(input.columns, input.rows).ok_or(TerminalError::TerminalNotFound)?;
        entry
            .session
            .resize(size)
            .await
            .map_err(|_ended| TerminalError::TerminalEnded)
    }

    fn acknowledge(&self, input: AcknowledgeOutput) -> Result<(), TerminalError> {
        let entry = self.entry(&input.conversation_id.to_string(), input.terminal)?;
        entry.session.acknowledge(input.view, input.seq.0);
        Ok(())
    }

    async fn block_text(&self, input: BlockRef) -> Result<BlockTextView, TerminalError> {
        let entry = self.entry(&input.conversation_id.to_string(), input.terminal)?;
        let number = input.block;
        let (block, text) = entry
            .session
            .ask(move |core| core.block_text(number))
            .await
            .flatten()
            .ok_or(TerminalError::BlockNotFound)?;
        Ok(BlockTextView {
            command: block.command,
            lines: text.lines,
            omitted: u32::try_from(text.omitted).unwrap_or(u32::MAX),
        })
    }

    async fn mention_block(&self, input: BlockRef) -> Result<MentionEntry, TerminalError> {
        let entry = self.entry(&input.conversation_id.to_string(), input.terminal)?;
        let number = input.block;
        let blocks = entry
            .session
            .ask(|core| core.blocks())
            .await
            .unwrap_or_default();
        let block = blocks
            .iter()
            .find(|block| block.number == number)
            .ok_or(TerminalError::BlockNotFound)?;
        Ok(block_entry(&entry, block))
    }

    async fn mention_selection(
        &self,
        input: MentionSelection,
    ) -> Result<MentionEntry, TerminalError> {
        let entry = self.entry(&input.conversation_id.to_string(), input.terminal)?;
        let text = input.text;
        let kept = text.clone();
        let key = entry
            .session
            .ask(move |core| core.keep_selection(kept))
            .await
            .ok_or(TerminalError::TerminalEnded)?;
        Ok(selection_entry(&entry, key, &text))
    }

    async fn update_settings(
        &self,
        input: TerminalSettings,
    ) -> Result<TerminalSettings, TerminalError> {
        self.settings.write(&input).await.map_err(|error| {
            tracing::warn!(
                error.code = error.code(),
                "the terminal settings could not be stored"
            );
            TerminalError::StorageFailed
        })?;
        self.terminals
            .set_default_shell(input.default_shell.clone().map(PathBuf::from));
        Ok(input)
    }

    fn follow_terminal(
        &self,
        input: FollowTerminal,
    ) -> Result<PluginStream<TerminalUpdate>, TerminalError> {
        let entry = self.entry(&input.conversation_id.to_string(), input.terminal)?;
        let (sender, receiver) = tokio::sync::mpsc::channel(VIEW_QUEUE);
        let scrollback = usize::try_from(input.scrollback).unwrap_or(usize::MAX);
        entry
            .session
            .subscribe(View::new(input.view, sender, scrollback));
        Ok(stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|update| (update, receiver))
        })
        .boxed())
    }
}

/// The handlers of a start.
fn handlers(shared: &Shared) -> Handlers {
    let opening = shared.clone();
    let writing = shared.clone();
    let resizing = shared.clone();
    let ending = shared.clone();
    let acknowledging = shared.clone();
    let reading = shared.clone();
    let mentioning = shared.clone();
    let selecting = shared.clone();
    let getting = shared.clone();
    let updating = shared.clone();
    let hosts = shared.clone();
    let lists = shared.clone();
    let views = shared.clone();
    Handlers::new()
        .command::<OpenTerminalCommand, _, _>(move |input: OpenTerminal| {
            let shared = opening.clone();
            async move { shared.open(input).await }
        })
        .command::<WriteTerminalCommand, _, _>(move |input: WriteTerminal| {
            let shared = writing.clone();
            async move { shared.write(input).await }
        })
        .command::<ResizeTerminalCommand, _, _>(move |input: ResizeTerminal| {
            let shared = resizing.clone();
            async move { shared.resize(input).await }
        })
        .command::<EndTerminalCommand, _, _>(move |input: TerminalRef| {
            let shared = ending.clone();
            async move {
                shared
                    .terminals
                    .end(&input.conversation_id.to_string(), input.terminal)
                    .await
            }
        })
        .command::<AcknowledgeOutputCommand, _, _>(move |input: AcknowledgeOutput| {
            std::future::ready(acknowledging.acknowledge(input))
        })
        .command::<BlockTextCommand, _, _>(move |input: BlockRef| {
            let shared = reading.clone();
            async move { shared.block_text(input).await }
        })
        .command::<MentionBlockCommand, _, _>(move |input: BlockRef| {
            let shared = mentioning.clone();
            async move { shared.mention_block(input).await }
        })
        .command::<MentionSelectionCommand, _, _>(move |input: MentionSelection| {
            let shared = selecting.clone();
            async move { shared.mention_selection(input).await }
        })
        .command::<GetSettingsCommand, _, _>(move |()| {
            let shared = getting.clone();
            async move {
                shared.settings.read().await.map_err(|error| {
                    tracing::warn!(
                        error.code = error.code(),
                        "the terminal settings could not be read"
                    );
                    TerminalError::StorageFailed
                })
            }
        })
        .command::<UpdateSettingsCommand, _, _>(move |input: TerminalSettings| {
            let shared = updating.clone();
            async move { shared.update_settings(input).await }
        })
        .channel::<FollowHostsChannel, _, _>(move |()| {
            std::future::ready(Ok::<_, TerminalError>(hosts.terminals.follow_hosts()))
        })
        .channel::<FollowTerminalsChannel, _, _>(move |input: ConversationRef| {
            std::future::ready(Ok::<_, TerminalError>(
                lists.terminals.follow(input.conversation_id.to_string()),
            ))
        })
        .channel::<FollowTerminalChannel, _, _>(move |input: FollowTerminal| {
            std::future::ready(views.follow_terminal(input))
        })
}

/// Follows the core's hosts until the start stops, which drops the sender
/// of `stopped`.
async fn follow_hosts(
    service: Arc<dyn mannie_plugin_sdk::services::terminals::TerminalsApi>,
    sender: watch::Sender<TerminalHostList>,
    mut stopped: oneshot::Receiver<()>,
) {
    let mut hosts = service.hosts();
    loop {
        tokio::select! {
            _stopped = &mut stopped => return,
            list = hosts.next() => match list {
                Some(list) => {
                    sender.send_replace(list);
                }
                None => return,
            },
        }
    }
}

impl TerminalPlugin {
    async fn start_now(context: StartContext) -> Result<Started, StartFailure> {
        let store = context.service::<Store>()?;
        let service = context.service::<TerminalsService>()?;
        let settings = SettingsStore::new(store);
        let stored = settings.read().await.map_err(|error| {
            tracing::error!(
                error.code = error.code(),
                "the terminal plugin could not read its settings"
            );
            StartFailure::new(STORAGE_FAILED)
        })?;
        let (hosts_sender, hosts) = watch::channel(TerminalHostList::default());
        let (stop_hosts, stopped) = oneshot::channel();
        tokio::spawn(follow_hosts(Arc::clone(&service), hosts_sender, stopped));
        let terminals = Terminals::new(service, context.clock(), hosts);
        terminals.set_default_shell(stored.default_shell.map(PathBuf::from));
        let agents: Vec<String> = AGENTS.iter().map(|agent| (*agent).to_owned()).collect();
        let run = PluginTool::new(
            Arc::new(RunInTerminal::new(terminals.clone())),
            AgentSelector::Agents(agents.clone()),
        )
        .with_side_effects();
        let list = PluginTool::new(
            Arc::new(ListTerminals::new(terminals.clone())),
            AgentSelector::Agents(agents),
        );
        let section = Section::new(
            SECTION_ID,
            SECTION_POSITION,
            SectionBody::Source(Arc::new(TerminalsSection::new(terminals.clone()))),
        );
        let mentions: Arc<dyn MentionProvider> = Arc::new(CommandMentions::new(terminals.clone()));
        let contributions = Contributions::new()
            .add::<Tools>(run)
            .add::<Tools>(list)
            .add::<Sections>(section)
            .add::<Mentions>(mentions);
        let shared = Shared {
            terminals: terminals.clone(),
            settings,
        };
        tracing::info!("the terminal plugin started");
        Ok(Started::new(TerminalInstance {
            terminals,
            stop_hosts,
        })
        .with_contributions(contributions)
        .with_handlers(handlers(&shared)))
    }
}

impl Plugin for TerminalPlugin {
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

/// The plugin while it is on.
#[derive(Debug)]
struct TerminalInstance {
    terminals: Terminals,
    stop_hosts: oneshot::Sender<()>,
}

impl Instance for TerminalInstance {
    fn work(&self) -> BoxFuture<'static, Vec<WorkItem>> {
        let running = self.terminals.running_commands();
        let work = if running == 0 {
            Vec::new()
        } else {
            vec![WorkItem::new(RUNNING_COMMANDS, running)]
        };
        Box::pin(std::future::ready(work))
    }

    fn stop(self: Box<Self>) -> BoxFuture<'static, ()> {
        let Self {
            terminals,
            stop_hosts,
        } = *self;
        drop(stop_hosts);
        Box::pin(async move {
            terminals.end_all().await;
            tracing::info!("the terminal plugin stopped");
        })
    }
}
