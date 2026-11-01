//! The plugin's terminals, by conversation (plugins/terminal/docs/README.md):
//! opening them through the core's service `terminals`, numbering them, the
//! lists that the toolbar control follows, and ending them.
//!
//! Each terminal belongs to the conversation that opened it (D45). Numbers
//! grow within a conversation and are never reused while the plugin runs.
//! A terminal whose conversation was deleted leaves the list; one that ended
//! otherwise stays, so its output can still be read, until the owner removes
//! it or 8 more ended after it.
//!
//! A host's plugin may learn the host's login shell only as the first
//! terminal there opens, and lists it then. So the setup of a remote
//! terminal is chosen when it is typed, after the first prompt, from the
//! shell that the plugin lists by then.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use futures_util::{StreamExt as _, stream};
use mannie_plugin_sdk::contracts::Clock;
use mannie_plugin_sdk::ipc::PluginStream;
use mannie_plugin_sdk::services::terminals::{
    ExpectedHost, LocalShell, OfferedHost, ShellKind, TerminalEnding, TerminalHostList,
    TerminalHostRef, TerminalHostState, TerminalOpenError, TerminalSize, TerminalSpec,
    TerminalsApi,
};
use tokio::sync::watch;

use crate::blocks::Author;
use crate::cite;
use crate::commands::{
    HostStateView, HostsView, OfferedHostView, ShellView, TerminalError, TerminalHostView,
    TerminalListUpdate, TerminalPhase, TerminalSummary,
};
use crate::marks;
use crate::session::{Session, SessionStart, ending_view};
use crate::{setup, texts};

/// The most ended terminals that a conversation keeps.
const MAX_ENDED: usize = 8;

/// How long the output of a remote host's first prompt must pause before
/// the plugin types the setup.
const SETUP_QUIET: Duration = Duration::from_millis(300);

/// The longest that the plugin waits for a remote host's first prompt.
const SETUP_WAIT: Duration = Duration::from_secs(10);

/// The longest that the plugin waits, after the first prompt, for a host's
/// plugin to list the host's login shell.
const SHELL_WAIT: Duration = Duration::from_secs(2);

/// Where a terminal runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionHost {
    /// This computer, in a shell.
    ThisComputer {
        /// The shell's program.
        program: PathBuf,
    },
    /// A host that a plugin offers.
    Remote {
        /// The plugin.
        plugin: String,
        /// The plugin's key for the host.
        key: String,
        /// The host's name.
        label: String,
        /// Where the terminal runs.
        endpoint: String,
    },
}

impl SessionHost {
    /// The host as the core's service names it.
    pub(crate) fn host_ref(&self) -> TerminalHostRef {
        match self {
            Self::ThisComputer { program } => TerminalHostRef::ThisMachine {
                shell: program.clone(),
            },
            Self::Remote { plugin, key, .. } => TerminalHostRef::Plugin {
                plugin: plugin.clone(),
                key: key.clone(),
            },
        }
    }

    /// The host's name, as the model reads it.
    pub(crate) fn label(&self) -> &str {
        match self {
            Self::ThisComputer { .. } => "this computer",
            Self::Remote { label, .. } => label,
        }
    }
}

/// A terminal of a conversation.
#[derive(Debug)]
pub(crate) struct Entry {
    /// Its number in the conversation.
    pub(crate) number: u32,
    /// The conversation.
    pub(crate) conversation: String,
    /// Its session.
    pub(crate) session: Arc<Session>,
    /// Where it runs.
    pub(crate) host: SessionHost,
    /// Its shell's name, or `None` for a host's login shell that the host's
    /// plugin has not listed yet.
    shell: Mutex<Option<String>>,
    /// Who opened it.
    pub(crate) opened_by: Author,
}

impl Entry {
    /// Its shell's name, as the model reads it.
    pub(crate) fn shell(&self) -> String {
        self.shell_name()
            .unwrap_or_else(|| texts::LOGIN_SHELL.to_owned())
    }

    fn shell_name(&self) -> Option<String> {
        self.shell
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The terminal as the lists show it.
    pub(crate) fn summary(&self) -> TerminalSummary {
        let status = self.session.status();
        let host = match &self.host {
            SessionHost::ThisComputer { program } => TerminalHostView::ThisComputer {
                program: program.to_string_lossy().into_owned(),
            },
            SessionHost::Remote {
                plugin,
                key,
                label,
                endpoint,
            } => TerminalHostView::Remote {
                plugin: plugin.clone(),
                key: key.clone(),
                label: label.clone(),
                endpoint: endpoint.clone(),
            },
        };
        TerminalSummary {
            number: self.number,
            session: self.session.id.clone(),
            host,
            shell: self.shell_name().unwrap_or_default(),
            opened_by: self.opened_by,
            phase: status.phase,
            marking: status.marking,
            state: status.state,
            ending: status.ending.as_ref().map(ending_view),
        }
    }

    /// Whether it ended.
    pub(crate) fn ended(&self) -> bool {
        self.session.status().phase == TerminalPhase::Ended
    }
}

/// Where a terminal opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HostChoice {
    /// This computer, in a shell by its program, or the default shell.
    ThisComputer {
        /// The shell's program.
        shell: Option<PathBuf>,
    },
    /// A host that a plugin offers.
    Plugin {
        /// The plugin.
        plugin: String,
        /// The plugin's key for the host.
        key: String,
    },
}

/// A terminal to open.
#[derive(Debug, Clone)]
pub(crate) struct OpenRequest {
    /// The conversation.
    pub(crate) conversation: String,
    /// Where it opens.
    pub(crate) host: HostChoice,
    /// The folder that a terminal on this computer starts in.
    pub(crate) folder: Option<PathBuf>,
    /// Its size.
    pub(crate) size: TerminalSize,
    /// Who opens it.
    pub(crate) opened_by: Author,
    /// For an approved command, the endpoint and identity that the approval
    /// named.
    pub(crate) expect: Option<ExpectedHost>,
}

#[derive(Debug, Default)]
struct Conversation {
    next_number: u32,
    terminals: BTreeMap<u32, Arc<Entry>>,
}

#[derive(Debug)]
struct Inner {
    service: Arc<dyn TerminalsApi>,
    clock: Arc<dyn Clock>,
    hosts: watch::Receiver<TerminalHostList>,
    default_shell: Mutex<Option<PathBuf>>,
    conversations: Mutex<HashMap<String, Conversation>>,
    /// Bumped whenever a list of terminals or a terminal's state changes.
    lists: watch::Sender<u64>,
    running: Arc<AtomicUsize>,
}

/// The plugin's terminals.
#[derive(Debug, Clone)]
pub(crate) struct Terminals {
    inner: Arc<Inner>,
}

impl Terminals {
    /// The terminals of a start, which follows the core's hosts through
    /// `hosts`.
    pub(crate) fn new(
        service: Arc<dyn TerminalsApi>,
        clock: Arc<dyn Clock>,
        hosts: watch::Receiver<TerminalHostList>,
    ) -> Self {
        let (lists, _receiver) = watch::channel(0);
        Self {
            inner: Arc::new(Inner {
                service,
                clock,
                hosts,
                default_shell: Mutex::new(None),
                conversations: Mutex::new(HashMap::new()),
                lists,
                running: Arc::new(AtomicUsize::new(0)),
            }),
        }
    }

    fn conversations(&self) -> std::sync::MutexGuard<'_, HashMap<String, Conversation>> {
        self.inner
            .conversations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Sets the shell that a terminal on this computer runs when none is
    /// chosen, or `None` for the system's default.
    pub(crate) fn set_default_shell(&self, shell: Option<PathBuf>) {
        *self
            .inner
            .default_shell
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = shell;
    }

    /// Where terminals open now.
    pub(crate) fn hosts(&self) -> TerminalHostList {
        self.inner.hosts.borrow().clone()
    }

    /// Where terminals open, each item the complete list.
    pub(crate) fn follow_hosts(&self) -> PluginStream<HostsView> {
        stream::unfold(
            (self.inner.hosts.clone(), true),
            |(mut hosts, first)| async move {
                if !first && hosts.changed().await.is_err() {
                    return None;
                }
                let view = hosts_view(&hosts.borrow_and_update());
                Some((view, (hosts, false)))
            },
        )
        .boxed()
    }

    /// The shell of this computer that `program` names, or the default.
    pub(crate) fn local_shell(&self, program: Option<&PathBuf>) -> Option<LocalShell> {
        let hosts = self.inner.hosts.borrow();
        if let Some(program) = program {
            return hosts
                .shells
                .iter()
                .find(|shell| shell.program == *program)
                .cloned();
        }
        let preferred = self
            .inner
            .default_shell
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let chosen = preferred.and_then(|preferred| {
            hosts
                .shells
                .iter()
                .find(|shell| shell.program == preferred)
                .cloned()
        });
        chosen.or_else(|| {
            hosts
                .shells
                .iter()
                .find(|shell| shell.default)
                .or_else(|| hosts.shells.first())
                .cloned()
        })
    }

    /// The host that a plugin offers under `key`.
    pub(crate) fn offered_host(&self, plugin: &str, key: &str) -> Option<OfferedHost> {
        self.inner
            .hosts
            .borrow()
            .hosts
            .iter()
            .find(|host| host.plugin == plugin && host.key == key)
            .cloned()
    }

    /// Opens a terminal.
    pub(crate) async fn open(&self, request: OpenRequest) -> Result<Arc<Entry>, TerminalError> {
        let (host, shell) = self.resolve(&request.host)?;
        let nonce = marks::new_nonce().ok_or_else(|| TerminalError::OpenFailed {
            reason: "no_random".to_owned(),
        })?;
        let mut spec =
            TerminalSpec::new(request.conversation.clone(), host.host_ref(), request.size);
        if let (SessionHost::ThisComputer { .. }, Some(folder)) = (&host, &request.folder) {
            spec = spec.in_folder(folder.clone());
        }
        if let Some(kind) = shell.local_kind
            && let Some(text) = setup::injected(kind, &nonce)
        {
            spec = spec.with_setup(text);
        }
        if let Some(expect) = request.expect.clone() {
            spec = spec.expecting(expect);
        }
        let terminal = self.inner.service.open(spec).await.map_err(open_error)?;
        let session = Session::start(
            terminal,
            SessionStart {
                id: cite::new_terminal_id(),
                nonce: nonce.clone(),
                size: request.size,
                clock: Arc::clone(&self.inner.clock),
                running: Arc::clone(&self.inner.running),
            },
        );
        let entry = {
            let mut conversations = self.conversations();
            let conversation = conversations
                .entry(request.conversation.clone())
                .or_default();
            conversation.next_number += 1;
            let entry = Arc::new(Entry {
                number: conversation.next_number,
                conversation: request.conversation.clone(),
                session: Arc::clone(&session),
                host,
                shell: Mutex::new(shell.name),
                opened_by: request.opened_by,
            });
            conversation
                .terminals
                .insert(entry.number, Arc::clone(&entry));
            forget_old_ended(conversation);
            entry
        };
        tracing::info!(terminal = entry.number, by = ?request.opened_by, "a terminal opened");
        if shell.local_kind.is_none() {
            tokio::spawn(type_setup(self.clone(), Arc::clone(&entry), nonce));
        }
        tokio::spawn(watch_session(self.clone(), Arc::clone(&entry)));
        self.bump();
        Ok(entry)
    }

    /// Where a terminal opens, and its shell.
    fn resolve(&self, choice: &HostChoice) -> Result<(SessionHost, OpenedShell), TerminalError> {
        match choice {
            HostChoice::ThisComputer { shell } => {
                let local = self
                    .local_shell(shell.as_ref())
                    .ok_or(TerminalError::ShellNotListed)?;
                Ok((
                    SessionHost::ThisComputer {
                        program: local.program.clone(),
                    },
                    OpenedShell {
                        name: Some(local.name),
                        local_kind: Some(local.kind),
                    },
                ))
            }
            HostChoice::Plugin { plugin, key } => {
                let offered = self
                    .offered_host(plugin, key)
                    .ok_or(TerminalError::HostNotFound)?;
                if let TerminalHostState::Unavailable { code } = &offered.state {
                    return Err(TerminalError::HostUnavailable {
                        reason: code.clone(),
                    });
                }
                Ok((
                    SessionHost::Remote {
                        plugin: offered.plugin,
                        key: offered.key,
                        label: offered.label,
                        endpoint: offered.endpoint,
                    },
                    OpenedShell {
                        name: offered
                            .shell
                            .map(|kind| crate::commands::shell_kind_name(kind).to_owned()),
                        local_kind: None,
                    },
                ))
            }
        }
    }

    /// The login shell that a plugin lists for its host, once it does, or
    /// `None` when it lists none within [`SHELL_WAIT`] or no longer offers
    /// the host.
    async fn login_shell(&self, plugin: &str, key: &str) -> Option<ShellKind> {
        let mut hosts = self.inner.hosts.clone();
        let deadline = Instant::now() + SHELL_WAIT;
        loop {
            let offered = hosts
                .borrow_and_update()
                .hosts
                .iter()
                .find(|host| host.plugin == plugin && host.key == key)
                .map(|host| host.shell);
            match offered {
                None => return None,
                Some(Some(kind)) => return Some(kind),
                Some(None) => {}
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            match tokio::time::timeout(remaining, hosts.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) | Err(_) => return None,
            }
        }
    }

    /// The terminal with this number in a conversation.
    pub(crate) fn entry(&self, conversation: &str, number: u32) -> Option<Arc<Entry>> {
        self.conversations()
            .get(conversation)?
            .terminals
            .get(&number)
            .cloned()
    }

    /// The terminal of a conversation whose ID is `id`.
    pub(crate) fn by_id(&self, conversation: &str, id: &str) -> Option<Arc<Entry>> {
        self.conversations()
            .get(conversation)?
            .terminals
            .values()
            .find(|entry| entry.session.id == id)
            .cloned()
    }

    /// A conversation's terminals, by number.
    pub(crate) fn of(&self, conversation: &str) -> Vec<Arc<Entry>> {
        self.conversations()
            .get(conversation)
            .map(|conversation| conversation.terminals.values().cloned().collect())
            .unwrap_or_default()
    }

    /// An agent's own terminal on a host, at an idle prompt that marks
    /// commands, which its next command reuses.
    pub(crate) fn idle_agent_terminal(
        &self,
        conversation: &str,
        host: &SessionHost,
    ) -> Option<Arc<Entry>> {
        self.of(conversation).into_iter().rev().find(|entry| {
            let status = entry.session.status();
            entry.opened_by == Author::Agent
                && entry.host == *host
                && status.marking
                && status.phase == TerminalPhase::Prompt
        })
    }

    /// Ends a terminal, or removes one that ended.
    pub(crate) async fn end(&self, conversation: &str, number: u32) -> Result<(), TerminalError> {
        let entry = self
            .entry(conversation, number)
            .ok_or(TerminalError::TerminalNotFound)?;
        if entry.ended() {
            self.remove(conversation, number);
            return Ok(());
        }
        entry.session.end().await;
        tracing::info!(terminal = number, "the owner ended a terminal");
        Ok(())
    }

    /// Takes a terminal out of the lists and stops its thread.
    fn remove(&self, conversation: &str, number: u32) {
        let removed = {
            let mut conversations = self.conversations();
            let removed = conversations
                .get_mut(conversation)
                .and_then(|terminals| terminals.terminals.remove(&number));
            if conversations
                .get(conversation)
                .is_some_and(|terminals| terminals.terminals.is_empty())
            {
                conversations.remove(conversation);
            }
            removed
        };
        if let Some(entry) = removed {
            entry.session.stop();
            self.bump();
        }
    }

    /// How many terminals run a command, for the plugin's work in progress.
    pub(crate) fn running_commands(&self) -> u32 {
        let count = self
            .conversations()
            .values()
            .flat_map(|conversation| conversation.terminals.values())
            .filter(|entry| entry.session.status().phase == TerminalPhase::Running)
            .count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    /// Ends every terminal and stops their threads, when the plugin stops.
    pub(crate) async fn end_all(&self) {
        let entries: Vec<Arc<Entry>> = {
            let mut conversations = self.conversations();
            conversations
                .drain()
                .flat_map(|(_id, conversation)| conversation.terminals.into_values())
                .collect()
        };
        let endings = entries.iter().map(|entry| entry.session.end());
        futures_util::future::join_all(endings).await;
        for entry in entries {
            entry.session.stop();
        }
        self.bump();
    }

    fn bump(&self) {
        self.inner.lists.send_modify(|version| *version += 1);
    }

    /// The terminals of a conversation, each item the complete list, sent
    /// again whenever it changes.
    pub(crate) fn follow(&self, conversation: String) -> PluginStream<TerminalListUpdate> {
        let terminals = self.clone();
        let changes = self.inner.lists.subscribe();
        stream::unfold(
            (terminals, changes, conversation, None::<TerminalListUpdate>),
            |(terminals, mut changes, conversation, last)| async move {
                loop {
                    let update = TerminalListUpdate {
                        terminals: terminals
                            .of(&conversation)
                            .iter()
                            .map(|entry| entry.summary())
                            .collect(),
                    };
                    if last.as_ref() != Some(&update) {
                        return Some((
                            update.clone(),
                            (terminals, changes, conversation, Some(update)),
                        ));
                    }
                    if changes.changed().await.is_err() {
                        return None;
                    }
                }
            },
        )
        .boxed()
    }
}

/// Lets go of the oldest ended terminals beyond [`MAX_ENDED`].
fn forget_old_ended(conversation: &mut Conversation) {
    let ended: Vec<u32> = conversation
        .terminals
        .values()
        .filter(|entry| entry.ended())
        .map(|entry| entry.number)
        .collect();
    for number in ended.iter().take(ended.len().saturating_sub(MAX_ENDED)) {
        if let Some(entry) = conversation.terminals.remove(number) {
            entry.session.stop();
        }
    }
}

/// Follows a terminal's state for the lists, and takes it out of them when
/// its conversation was deleted.
async fn watch_session(terminals: Terminals, entry: Arc<Entry>) {
    let mut status = entry.session.follow_status();
    while status.changed().await.is_ok() {
        let deleted = matches!(
            status.borrow().ending,
            Some(TerminalEnding::ConversationDeleted)
        );
        terminals.bump();
        if deleted {
            terminals.remove(&entry.conversation, entry.number);
            return;
        }
    }
}

/// The shell of a terminal that opens: its name, when it is known, and its
/// kind, for a shell of this computer, whose setup the core injects.
#[derive(Debug)]
struct OpenedShell {
    name: Option<String>,
    local_kind: Option<ShellKind>,
}

/// Types a remote host's setup, for the login shell that the host's plugin
/// lists, once the first prompt's output paused. A shell that the plugin
/// does not name, or that no setup reaches, leaves the terminal plain.
async fn type_setup(terminals: Terminals, entry: Arc<Entry>, nonce: String) {
    let SessionHost::Remote { plugin, key, .. } = &entry.host else {
        return;
    };
    let session = &entry.session;
    let mut status = session.follow_status();
    while status.borrow().outputs == 0 {
        if status.changed().await.is_err() {
            return;
        }
    }
    let deadline = Instant::now() + SETUP_WAIT;
    loop {
        match tokio::time::timeout(SETUP_QUIET, status.changed()).await {
            Ok(Ok(())) if Instant::now() < deadline => {}
            Ok(Err(_ended)) => return,
            Ok(Ok(())) | Err(_) => break,
        }
    }
    if status.borrow().ending.is_some() {
        return;
    }
    let Some(kind) = terminals.login_shell(plugin, key).await else {
        tracing::info!(
            terminal = entry.number,
            "the login shell of a remote host is not known, so its terminal stays plain"
        );
        return;
    };
    *entry.shell.lock().unwrap_or_else(PoisonError::into_inner) =
        Some(crate::commands::shell_kind_name(kind).to_owned());
    terminals.bump();
    let Some(line) = setup::typed(kind, &nonce) else {
        tracing::info!(
            terminal = entry.number,
            ?kind,
            "no setup reaches the login shell of a remote host, so its terminal stays plain"
        );
        return;
    };
    if let Err(error) = session.write(line.into_bytes()).await {
        tracing::warn!(error.code = ?error, "the setup of a remote terminal could not be typed");
    }
}

/// Where terminals open, as the UI shows it.
fn hosts_view(list: &TerminalHostList) -> HostsView {
    HostsView {
        shells: list
            .shells
            .iter()
            .map(|shell| ShellView {
                program: shell.program.to_string_lossy().into_owned(),
                name: shell.name.clone(),
                default: shell.default,
            })
            .collect(),
        hosts: list
            .hosts
            .iter()
            .map(|host| OfferedHostView {
                plugin: host.plugin.clone(),
                key: host.key.clone(),
                label: host.label.clone(),
                endpoint: host.endpoint.clone(),
                state: match &host.state {
                    TerminalHostState::Unavailable { code } => {
                        HostStateView::Unavailable { code: code.clone() }
                    }
                    _ => HostStateView::Ready,
                },
            })
            .collect(),
    }
}

/// Why the core did not open a terminal, as the plugin's error.
fn open_error(error: TerminalOpenError) -> TerminalError {
    match error {
        TerminalOpenError::ConversationNotFound => TerminalError::ConversationNotFound,
        TerminalOpenError::ShellNotListed => TerminalError::ShellNotListed,
        TerminalOpenError::HostNotFound => TerminalError::HostNotFound,
        TerminalOpenError::HostChanged => TerminalError::HostUnavailable {
            reason: "host_changed".to_owned(),
        },
        TerminalOpenError::HostUnavailable { reason } => TerminalError::HostUnavailable { reason },
        TerminalOpenError::FolderInvalid => TerminalError::FolderInvalid,
        TerminalOpenError::TooMany { limit } => TerminalError::TooMany { limit },
        TerminalOpenError::Unsupported => TerminalError::Unsupported,
        TerminalOpenError::ShuttingDown => TerminalError::OpenFailed {
            reason: "shutting_down".to_owned(),
        },
        TerminalOpenError::SetupRefused => TerminalError::OpenFailed {
            reason: "setup_refused".to_owned(),
        },
        TerminalOpenError::Failed { reason } => TerminalError::OpenFailed { reason },
        _ => TerminalError::OpenFailed {
            reason: "unknown".to_owned(),
        },
    }
}
