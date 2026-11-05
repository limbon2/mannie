//! The runtime's registry of terminals (ADR-0053, decisions 3 and 4): the
//! hosts that plugins offer through the point `terminal-hosts`, this
//! machine's shells, and the terminals that run, which the service
//! `terminals` opens for a plugin.
//!
//! A terminal on this machine is a shell in a pseudo console under the
//! process supervisor; a terminal on a host is the connection that the
//! host's source opened. Each terminal has a task of its own, which hands its
//! output to the plugin, writes what the plugin types, and ends it. Every
//! terminal belongs to a conversation: deleting the conversation, the end of
//! the plugin that opened it and Mannie quitting end it, and its output ends
//! with how it ended.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::stream::{self, BoxStream as FuturesBoxStream};
use futures_util::{FutureExt, StreamExt};
use mannie_plugin_sdk::host::{BindRefusal, PluginBinding, ServiceProvider};
use mannie_plugin_sdk::ids::Contributor;
use mannie_plugin_sdk::points::terminal_hosts::{
    MAX_SOURCE_HOSTS, OpenTerminal, PluginTerminalHostSource, TerminalConnection,
    TerminalHostEntry, TerminalHostSource, TerminalOpenFailure, check_host_entry,
};
use mannie_plugin_sdk::service::{BoundService, Service};
use mannie_plugin_sdk::services::terminals::{
    LocalShell as SdkLocalShell, MAX_SETUP_BYTES, MAX_TERMINAL_CLOSE_GRACE,
    MAX_TERMINAL_WRITE_BYTES, MAX_TERMINALS_PER_CONVERSATION, MAX_TERMINALS_PER_MACHINE,
    OfferedHost, ShellKind as SdkShellKind, Terminal, TerminalEnding, TerminalHostList,
    TerminalHostRef, TerminalHostState, TerminalIoError, TerminalOpenError, TerminalOutput,
    TerminalSize, TerminalSpec, Terminals as TerminalsService, TerminalsApi,
};
use mannie_plugin_sdk::{BoxFuture, BoxStream};
use mannie_tools::{
    CommandEnvironment, LocalShell, ShellKind, TerminalInput, TerminalLaunch, TerminalProcess,
    canonical_folder, local_shells, path_is_inside, shell_start,
};
use mannie_types::ids::ConversationId;
use mannie_types::tools::{TerminalHost, ToolsSettings};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::AbortHandle;

use crate::{Runtime, lock};

/// How many chunks of a terminal's output wait for its plugin. A plugin that
/// reads none holds the terminal back, as a full pipe would.
const OUTPUT_QUEUE: usize = 64;

/// How many requests of a plugin wait for its terminal's task.
const COMMAND_QUEUE: usize = 64;

/// How long a shell that exited on its own leaves its last output to arrive.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// How long the end of a plugin, of a conversation or of Mannie lets each
/// terminal's shell end after its hang-up.
const RELEASE_GRACE: Duration = Duration::from_secs(2);

/// How long the end of a plugin, of a conversation or of Mannie waits for
/// its terminals in all.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(5);

/// The terminals' state of the runtime.
#[derive(Debug)]
pub(crate) struct Terminals {
    /// The hosts of the plugins' sources, by source ID.
    sources: Mutex<BTreeMap<String, HostSource>>,
    /// A number that grows whenever the hosts change, which the lists of
    /// hosts watch.
    changed: watch::Sender<u64>,
    /// The terminals that run, by their own number.
    running: Mutex<HashMap<u64, Entry>>,
    next_id: AtomicU64,
}

impl Default for Terminals {
    fn default() -> Self {
        Self {
            sources: Mutex::default(),
            changed: watch::Sender::new(0),
            running: Mutex::default(),
            next_id: AtomicU64::new(1),
        }
    }
}

/// A plugin's source of hosts, with the hosts that it listed last.
#[derive(Debug)]
struct HostSource {
    plugin: String,
    /// The key of the contributor whose start offered the source: the
    /// plugin's ID, or the plugin's ID, a slash and its feature's ID.
    owner: String,
    source: Arc<dyn TerminalHostSource>,
    hosts: Vec<TerminalHostEntry>,
    drain: Option<AbortHandle>,
}

/// A terminal that runs, as the registry knows it.
#[derive(Debug)]
struct Entry {
    plugin: String,
    conversation: String,
    running: Arc<AtomicBool>,
    stop: mpsc::Sender<Request>,
}

/// What a terminal's task does next, at the request of its plugin or of the
/// core.
#[derive(Debug)]
enum Request {
    Write(Vec<u8>, oneshot::Sender<Result<(), TerminalIoError>>),
    Resize(TerminalSize, oneshot::Sender<Result<(), TerminalIoError>>),
    End {
        /// How the terminal ended, when the core ended it; `None` when its
        /// plugin closed it.
        reason: Option<TerminalEnding>,
        grace: Duration,
        ended: Option<oneshot::Sender<TerminalEnding>>,
    },
}

/// Which terminals an end reaches.
enum Which<'a> {
    Plugin(&'a str),
    Conversations(&'a [String]),
    All,
}

impl Terminals {
    fn bump(&self) {
        self.changed.send_modify(|version| *version += 1);
    }

    fn count(&self, conversation: Option<&str>) -> usize {
        let running = lock(&self.running);
        match conversation {
            Some(conversation) => running
                .values()
                .filter(|entry| entry.conversation == conversation)
                .count(),
            None => running.len(),
        }
    }

    /// Ends the terminals that `which` names, and waits for them for at most
    /// [`RELEASE_TIMEOUT`].
    async fn end(&self, which: Which<'_>, reason: TerminalEnding) {
        let stops: Vec<mpsc::Sender<Request>> = {
            let running = lock(&self.running);
            running
                .values()
                .filter(|entry| match &which {
                    Which::Plugin(plugin) => entry.plugin == *plugin,
                    Which::Conversations(conversations) => {
                        conversations.contains(&entry.conversation)
                    }
                    Which::All => true,
                })
                .map(|entry| entry.stop.clone())
                .collect()
        };
        if stops.is_empty() {
            return;
        }
        tracing::info!(terminals = stops.len(), ?reason, "ending terminals");
        let ended = stops.into_iter().map(|stop| {
            let reason = reason.clone();
            async move {
                let (sender, receiver) = oneshot::channel();
                let request = Request::End {
                    reason: Some(reason),
                    grace: RELEASE_GRACE,
                    ended: Some(sender),
                };
                if stop.send(request).await.is_ok() {
                    let _ending = receiver.await;
                }
            }
        });
        if tokio::time::timeout(RELEASE_TIMEOUT, futures_util::future::join_all(ended))
            .await
            .is_err()
        {
            tracing::warn!("terminals did not end in time");
        }
    }
}

impl Runtime {
    /// Takes each of a plugin's sources of terminal hosts and drains its
    /// stream in a task of its own, until the stream ends or the plugin
    /// stops (ADR-0053, decision 3). A stream that ends empties its source.
    pub(crate) fn add_terminal_sources(
        &self,
        contributor: &Contributor,
        sources: Vec<PluginTerminalHostSource>,
    ) {
        for source in sources {
            let id = source.id.clone();
            let mut hosts = source.source.hosts();
            let first = hosts.next().now_or_never().flatten();
            {
                let mut registry = lock(&self.inner.terminals.sources);
                registry.insert(
                    id.clone(),
                    HostSource {
                        plugin: contributor.plugin_id().as_str().to_owned(),
                        owner: contributor.key(),
                        source: Arc::clone(&source.source),
                        hosts: Vec::new(),
                        drain: None,
                    },
                );
            }
            if let Some(first) = first {
                self.list_terminal_hosts(&id, first);
            }
            let runtime = self.clone();
            let source_id = id.clone();
            let drain = tokio::spawn(async move {
                while let Some(hosts) = hosts.next().await {
                    runtime.list_terminal_hosts(&source_id, hosts);
                }
                runtime.list_terminal_hosts(&source_id, Vec::new());
            });
            if let Some(source) = lock(&self.inner.terminals.sources).get_mut(&id) {
                source.drain = Some(drain.abort_handle());
            }
        }
    }

    /// Keeps the hosts that a source listed and that keep the point's rules.
    fn list_terminal_hosts(&self, source: &str, hosts: Vec<TerminalHostEntry>) {
        let mut kept: Vec<TerminalHostEntry> = Vec::new();
        for host in hosts.into_iter().take(MAX_SOURCE_HOSTS) {
            match check_host_entry(&host) {
                Ok(()) if !kept.iter().any(|other| other.key == host.key) => kept.push(host),
                Ok(()) => tracing::warn!(source, key = host.key, "a terminal host's key repeats"),
                Err(field) => {
                    tracing::warn!(source, field, "a terminal host breaks the point's rules");
                }
            }
        }
        let terminals = &self.inner.terminals;
        if let Some(entry) = lock(&terminals.sources).get_mut(source) {
            entry.hosts = kept;
        }
        terminals.bump();
    }

    /// Forgets the sources of terminal hosts of a plugin or a feature, as it
    /// stops.
    pub(crate) fn remove_terminal_sources(&self, contributor: &Contributor) {
        let owner = contributor.key();
        let removed: Vec<HostSource> = {
            let mut sources = lock(&self.inner.terminals.sources);
            let ids: Vec<String> = sources
                .iter()
                .filter(|(_, source)| source.owner == owner)
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter()
                .filter_map(|id| sources.remove(&id))
                .collect()
        };
        if removed.is_empty() {
            return;
        }
        for source in &removed {
            if let Some(drain) = &source.drain {
                drain.abort();
            }
        }
        self.inner.terminals.bump();
    }

    /// Ends the terminals of conversations that were deleted.
    pub(crate) async fn end_conversation_terminals(&self, conversations: &[ConversationId]) {
        let ids: Vec<String> = conversations
            .iter()
            .map(|conversation| conversation.as_str().to_owned())
            .collect();
        self.inner
            .terminals
            .end(
                Which::Conversations(&ids),
                TerminalEnding::ConversationDeleted,
            )
            .await;
    }

    /// Ends every terminal, as Mannie quits.
    pub(crate) async fn end_terminals(&self) {
        self.inner
            .terminals
            .end(Which::All, TerminalEnding::ShuttingDown)
            .await;
    }

    /// How many terminals run a command now, as their plugins marked them,
    /// for the warning before Mannie quits (ADR-0053, decision 9).
    pub fn running_terminal_count(&self) -> u32 {
        let running = lock(&self.inner.terminals.running);
        let count = running
            .values()
            .filter(|entry| entry.running.load(Ordering::Relaxed))
            .count();
        u32::try_from(count).unwrap_or(u32::MAX)
    }

    /// The host of a terminal command as the policy decides on it: this
    /// machine in one of its listed shells, or a host that a plugin's source
    /// lists, with its name, endpoint and identity from the registry. `None`
    /// when no source lists the host or the shell is not listed.
    pub(crate) async fn resolve_terminal_host(
        &self,
        host: &TerminalHostRef,
    ) -> Option<(TerminalHost, String)> {
        match host {
            TerminalHostRef::ThisMachine { shell } => {
                let shells = self.local_terminal_shells().await;
                let listed = shells
                    .iter()
                    .find(|candidate| candidate.program == *shell)?;
                Some((
                    TerminalHost::ThisMachine {
                        shell: listed.program.to_string_lossy().into_owned(),
                    },
                    String::new(),
                ))
            }
            TerminalHostRef::Plugin { plugin, key } => {
                let (entry, _source) = self.offered_host(plugin, key)?;
                Some((
                    TerminalHost::Remote {
                        plugin: plugin.clone(),
                        target: entry.key,
                        label: entry.label,
                        endpoint: entry.endpoint,
                    },
                    entry.identity,
                ))
            }
        }
    }

    /// A host that a plugin's source lists, with the source.
    fn offered_host(
        &self,
        plugin: &str,
        key: &str,
    ) -> Option<(TerminalHostEntry, Arc<dyn TerminalHostSource>)> {
        let sources = lock(&self.inner.terminals.sources);
        sources
            .values()
            .filter(|source| source.plugin == plugin)
            .find_map(|source| {
                source
                    .hosts
                    .iter()
                    .find(|host| host.key == key)
                    .map(|host| (host.clone(), Arc::clone(&source.source)))
            })
    }

    /// This machine's shells, without programs inside the owner's projects
    /// or Mannie's own folders, which a model's changes could replace.
    async fn local_terminal_shells(&self) -> Vec<LocalShell> {
        let projects: Vec<PathBuf> = match self.inner.store.projects().await {
            Ok(projects) => projects
                .into_iter()
                .map(|project| PathBuf::from(project.folder))
                .collect(),
            Err(error) => {
                tracing::warn!(%error, "the projects could not be read to list the shells");
                Vec::new()
            }
        };
        let protected = self.inner.tools.protected.clone();
        tokio::task::spawn_blocking(move || {
            local_shells(&|program: &Path| {
                !projects
                    .iter()
                    .chain(protected.iter())
                    .any(|folder| path_is_inside(program, folder))
            })
        })
        .await
        .unwrap_or_default()
    }

    /// The list of hosts as the service gives it.
    async fn terminal_host_list(&self) -> TerminalHostList {
        let shells = self
            .local_terminal_shells()
            .await
            .into_iter()
            .map(|shell| {
                SdkLocalShell::new(
                    shell.program,
                    shell.name,
                    sdk_kind(shell.kind),
                    shell.default,
                )
            })
            .collect();
        let hosts = {
            let sources = lock(&self.inner.terminals.sources);
            sources
                .values()
                .flat_map(|source| {
                    source
                        .hosts
                        .iter()
                        .cloned()
                        .map(|host| OfferedHost::new(source.plugin.clone(), host))
                })
                .collect()
        };
        TerminalHostList::new(shells, hosts)
    }
}

fn sdk_kind(kind: ShellKind) -> SdkShellKind {
    match kind {
        ShellKind::Bash => SdkShellKind::Bash,
        ShellKind::Zsh => SdkShellKind::Zsh,
        ShellKind::Fish => SdkShellKind::Fish,
        ShellKind::Sh => SdkShellKind::Sh,
        ShellKind::PowerShell => SdkShellKind::PowerShell,
        ShellKind::Cmd => SdkShellKind::Cmd,
        ShellKind::Other => SdkShellKind::Other,
    }
}

/// The runtime's provider of the service `terminals`.
#[derive(Debug)]
pub(crate) struct TerminalsProvider {
    runtime: Runtime,
}

impl TerminalsProvider {
    /// The provider of `runtime`'s terminals.
    pub(crate) fn new(runtime: &Runtime) -> Self {
        Self {
            runtime: runtime.clone(),
        }
    }
}

impl ServiceProvider for TerminalsProvider {
    fn name(&self) -> &'static str {
        TerminalsService::NAME
    }

    fn major(&self) -> u16 {
        TerminalsService::MAJOR
    }

    fn bind(&self, binding: &PluginBinding<'_>) -> Result<BoundService, BindRefusal> {
        Ok(BoundService::new::<TerminalsService>(Arc::new(
            BoundTerminals {
                runtime: self.runtime.clone(),
                plugin: binding.contributor().key(),
            },
        )))
    }

    /// Ends the terminals that the plugin, or the feature, opened, once it
    /// stopped or failed to start, so none outlives it. A binding keeps the
    /// key of its contributor as the owner of the terminals that it opens.
    fn release(&self, contributor: &Contributor) -> BoxFuture<'static, ()> {
        let runtime = self.runtime.clone();
        let owner = contributor.key();
        async move {
            runtime
                .inner
                .terminals
                .end(Which::Plugin(&owner), TerminalEnding::PluginOff)
                .await;
        }
        .boxed()
    }
}

/// The service bound to one plugin.
#[derive(Debug)]
struct BoundTerminals {
    runtime: Runtime,
    plugin: String,
}

impl TerminalsApi for BoundTerminals {
    fn hosts(&self) -> BoxStream<'static, TerminalHostList> {
        let runtime = self.runtime.clone();
        let changes = runtime.inner.terminals.changed.subscribe();
        let lists: FuturesBoxStream<'static, TerminalHostList> = stream::unfold(
            (runtime, changes, true),
            |(runtime, mut changes, first)| async move {
                if !first && changes.changed().await.is_err() {
                    return None;
                }
                changes.borrow_and_update();
                let list = runtime.terminal_host_list().await;
                Some((list, (runtime, changes, false)))
            },
        )
        .boxed();
        lists
    }

    fn open(
        &self,
        spec: TerminalSpec,
    ) -> BoxFuture<'static, Result<Box<dyn Terminal>, TerminalOpenError>> {
        let runtime = self.runtime.clone();
        let plugin = self.plugin.clone();
        async move { open(runtime, plugin, spec).await }.boxed()
    }
}

/// Checks a terminal's spec and opens it.
async fn open(
    runtime: Runtime,
    plugin: String,
    spec: TerminalSpec,
) -> Result<Box<dyn Terminal>, TerminalOpenError> {
    if runtime.is_shutting_down() {
        return Err(TerminalOpenError::ShuttingDown);
    }
    let conversation = ConversationId::parse(&spec.conversation)
        .map_err(|_error| TerminalOpenError::ConversationNotFound)?;
    let exists = runtime
        .inner
        .store
        .conversation_summary(&conversation)
        .await
        .map_err(|error| {
            tracing::error!(%error, "a conversation could not be read to open a terminal");
            TerminalOpenError::Failed {
                reason: "store_failed".to_owned(),
            }
        })?
        .is_some();
    if !exists {
        return Err(TerminalOpenError::ConversationNotFound);
    }
    let terminals = &runtime.inner.terminals;
    if terminals.count(Some(&spec.conversation))
        >= usize::try_from(MAX_TERMINALS_PER_CONVERSATION).unwrap_or(usize::MAX)
    {
        return Err(TerminalOpenError::TooMany {
            limit: MAX_TERMINALS_PER_CONVERSATION,
        });
    }
    if terminals.count(None) >= usize::try_from(MAX_TERMINALS_PER_MACHINE).unwrap_or(usize::MAX) {
        return Err(TerminalOpenError::TooMany {
            limit: MAX_TERMINALS_PER_MACHINE,
        });
    }
    let backend = match &spec.host {
        TerminalHostRef::ThisMachine { shell } => open_local(&runtime, shell, &spec).await?,
        TerminalHostRef::Plugin {
            plugin: host_plugin,
            key,
        } => open_offered(&runtime, host_plugin, key, &spec).await?,
    };
    Ok(Box::new(RunningTerminal::start(
        &runtime,
        plugin,
        spec.conversation,
        backend,
    )))
}

/// Starts a listed shell of this machine in a pseudo console, with the
/// environment of commands and the plugin's setup.
async fn open_local(
    runtime: &Runtime,
    program: &Path,
    spec: &TerminalSpec,
) -> Result<Backend, TerminalOpenError> {
    let shells = runtime.local_terminal_shells().await;
    let shell = shells
        .into_iter()
        .find(|candidate| candidate.program == program)
        .ok_or(TerminalOpenError::ShellNotListed)?;
    if spec
        .setup
        .as_ref()
        .is_some_and(|setup| setup.len() > MAX_SETUP_BYTES)
    {
        return Err(TerminalOpenError::SetupRefused);
    }
    let working_directory = working_directory(runtime, spec.folder.as_deref())?;
    let settings = runtime
        .inner
        .store
        .load_settings::<ToolsSettings>()
        .await
        .unwrap_or_default();
    let mut environment: Vec<(OsString, OsString)> =
        CommandEnvironment::from_process(&settings.command_environment)
            .variables()
            .to_vec();
    environment.extend([
        ("TERM".into(), "xterm-256color".into()),
        ("COLORTERM".into(), "truecolor".into()),
        ("TERM_PROGRAM".into(), "Mannie".into()),
    ]);
    let setup_folder = SetupFolder::create()?;
    let start = shell_start(
        &shell,
        spec.setup.as_deref(),
        setup_folder.path(),
        &|name| std::env::var_os(name),
    )
    .map_err(|_refused| TerminalOpenError::SetupRefused)?;
    for (name, text) in &start.files {
        std::fs::write(setup_folder.path().join(name), text).map_err(|error| {
            tracing::warn!(%error, "a terminal's setup could not be written");
            TerminalOpenError::Failed {
                reason: "setup_failed".to_owned(),
            }
        })?;
    }
    environment.extend(start.environment);
    let launch = TerminalLaunch {
        program: shell.program.clone(),
        arguments: start.arguments,
        working_directory,
        environment,
        columns: spec.size.columns,
        rows: spec.size.rows,
    };
    let mut process = TerminalProcess::spawn(&launch).map_err(|error| {
        if error.kind() == std::io::ErrorKind::Unsupported {
            return TerminalOpenError::Unsupported;
        }
        tracing::warn!(shell = %shell.program.display(), %error, "a terminal's shell did not start");
        TerminalOpenError::Failed {
            reason: "spawn_failed".to_owned(),
        }
    })?;
    tracing::info!(shell = %shell.name, pid = process.record().pid, "a terminal started");
    let output = process
        .take_output()
        .ok_or_else(|| TerminalOpenError::Failed {
            reason: "spawn_failed".to_owned(),
        })?;
    let input = process.input();
    Ok(Backend::Local {
        process: Some(process),
        output,
        input,
        _setup: setup_folder,
    })
}

/// Opens a terminal on a host that a plugin's source lists, after the
/// approved endpoint and identity are checked against the registry.
async fn open_offered(
    runtime: &Runtime,
    plugin: &str,
    key: &str,
    spec: &TerminalSpec,
) -> Result<Backend, TerminalOpenError> {
    let (entry, source) = runtime
        .offered_host(plugin, key)
        .ok_or(TerminalOpenError::HostNotFound)?;
    if let TerminalHostState::Unavailable { code } = &entry.state {
        return Err(TerminalOpenError::HostUnavailable {
            reason: code.clone(),
        });
    }
    if let Some(expect) = &spec.expect
        && (expect.endpoint != entry.endpoint || expect.identity != entry.identity)
    {
        return Err(TerminalOpenError::HostChanged);
    }
    let request = OpenTerminal::new(entry.key.clone(), spec.size, spec.expect.clone());
    let mut connection = source
        .open(request)
        .await
        .map_err(|failure| match failure {
            TerminalOpenFailure::UnknownHost => TerminalOpenError::HostNotFound,
            TerminalOpenFailure::HostChanged => TerminalOpenError::HostChanged,
            TerminalOpenFailure::Failed { reason } => TerminalOpenError::Failed { reason },
            _ => TerminalOpenError::Failed {
                reason: "host_failed".to_owned(),
            },
        })?;
    tracing::info!(plugin, host = entry.label, "a terminal opened on a host");
    let output = connection.output();
    Ok(Backend::Offered {
        connection: Some(connection),
        output,
    })
}

/// The canonical folder that a shell starts in: an absolute folder that
/// exists, outside Mannie's own folders, or the owner's home folder.
fn working_directory(
    runtime: &Runtime,
    folder: Option<&Path>,
) -> Result<PathBuf, TerminalOpenError> {
    let folder = match folder {
        Some(folder) => folder.to_path_buf(),
        None => std::env::home_dir().ok_or(TerminalOpenError::FolderInvalid)?,
    };
    if !folder.is_absolute() {
        return Err(TerminalOpenError::FolderInvalid);
    }
    let canonical = PathBuf::from(
        canonical_folder(&folder).map_err(|_error| TerminalOpenError::FolderInvalid)?,
    );
    let in_mannie = runtime
        .inner
        .tools
        .protected
        .iter()
        .any(|mannie| path_is_inside(&canonical, mannie));
    if in_mannie {
        return Err(TerminalOpenError::FolderInvalid);
    }
    Ok(canonical)
}

/// The folder of a terminal's setup files, readable only by the owner's
/// account, which goes with the terminal.
#[derive(Debug)]
struct SetupFolder(PathBuf);

impl SetupFolder {
    fn create() -> Result<Self, TerminalOpenError> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        // The random part keeps the name unguessable in a shared folder.
        let unique = uuid::Uuid::now_v7().simple();
        let name = format!(
            "mannie-terminal-{}-{unique}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        // On Unix the temporary folder is shared, so only the owner's account
        // may enter this one; Windows gives each account its own.
        #[cfg(unix)]
        let builder = {
            use std::os::unix::fs::DirBuilderExt as _;
            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700);
            builder
        };
        #[cfg(not(unix))]
        let builder = std::fs::DirBuilder::new();
        // `create` fails when the folder exists, so nothing that another
        // account placed there first is used.
        builder.create(&path).map_err(|error| {
            tracing::warn!(%error, "a terminal's setup folder could not be made");
            TerminalOpenError::Failed {
                reason: "setup_failed".to_owned(),
            }
        })?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for SetupFolder {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            tracing::debug!(%error, "a terminal's setup folder was not removed");
        }
    }
}

/// What a terminal runs on.
enum Backend {
    /// A shell of this machine.
    Local {
        process: Option<TerminalProcess>,
        output: mpsc::Receiver<Vec<u8>>,
        input: TerminalInput,
        _setup: SetupFolder,
    },
    /// A connection that a host's source opened.
    Offered {
        connection: Option<Box<dyn TerminalConnection>>,
        output: BoxStream<'static, TerminalOutput>,
    },
}

/// A terminal that runs, as its plugin holds it.
#[derive(Debug)]
struct RunningTerminal {
    output: Option<mpsc::Receiver<TerminalOutput>>,
    requests: mpsc::Sender<Request>,
    running: Arc<AtomicBool>,
    closed: bool,
}

impl RunningTerminal {
    /// Hands the backend to a task of its own and registers the terminal.
    fn start(runtime: &Runtime, plugin: String, conversation: String, backend: Backend) -> Self {
        let (output_sender, output) = mpsc::channel(OUTPUT_QUEUE);
        let (requests, queue) = mpsc::channel(COMMAND_QUEUE);
        let running = Arc::new(AtomicBool::new(false));
        let terminals = &runtime.inner.terminals;
        let id = terminals.next_id.fetch_add(1, Ordering::Relaxed);
        lock(&terminals.running).insert(
            id,
            Entry {
                plugin,
                conversation,
                running: Arc::clone(&running),
                stop: requests.clone(),
            },
        );
        let runtime = runtime.clone();
        tokio::spawn(async move {
            let ending = serve(backend, queue, &output_sender).await;
            lock(&runtime.inner.terminals.running).remove(&id);
            tracing::info!(?ending, "a terminal ended");
            let _sent = output_sender.send(TerminalOutput::Ended { ending }).await;
        });
        Self {
            output: Some(output),
            requests,
            running,
            closed: false,
        }
    }

    async fn ask<T>(
        requests: mpsc::Sender<Request>,
        request: impl FnOnce(oneshot::Sender<Result<T, TerminalIoError>>) -> Request,
    ) -> Result<T, TerminalIoError> {
        let (sender, receiver) = oneshot::channel();
        requests
            .send(request(sender))
            .await
            .map_err(|_closed| TerminalIoError::Ended)?;
        receiver.await.map_err(|_dropped| TerminalIoError::Ended)?
    }
}

impl Terminal for RunningTerminal {
    fn output(&mut self) -> BoxStream<'static, TerminalOutput> {
        match self.output.take() {
            Some(output) => {
                let chunks: FuturesBoxStream<'static, TerminalOutput> =
                    stream::unfold(output, |mut output| async move {
                        output.recv().await.map(|item| (item, output))
                    })
                    .boxed();
                chunks
            }
            None => stream::empty().boxed(),
        }
    }

    fn write(&self, bytes: Vec<u8>) -> BoxFuture<'static, Result<(), TerminalIoError>> {
        let requests = self.requests.clone();
        async move {
            if bytes.len() > MAX_TERMINAL_WRITE_BYTES {
                return Err(TerminalIoError::OutOfLimits);
            }
            Self::ask(requests, |done| Request::Write(bytes, done)).await
        }
        .boxed()
    }

    fn resize(&self, size: TerminalSize) -> BoxFuture<'static, Result<(), TerminalIoError>> {
        let requests = self.requests.clone();
        async move {
            if TerminalSize::new(size.columns, size.rows).is_none() {
                return Err(TerminalIoError::OutOfLimits);
            }
            Self::ask(requests, |done| Request::Resize(size, done)).await
        }
        .boxed()
    }

    fn set_running(&self, running: bool) {
        self.running.store(running, Ordering::Relaxed);
    }

    fn close(mut self: Box<Self>, grace: Duration) -> BoxFuture<'static, TerminalEnding> {
        self.closed = true;
        let requests = self.requests.clone();
        async move {
            let (sender, receiver) = oneshot::channel();
            let request = Request::End {
                reason: None,
                grace: grace.min(MAX_TERMINAL_CLOSE_GRACE),
                ended: Some(sender),
            };
            if requests.send(request).await.is_err() {
                return TerminalEnding::Closed;
            }
            receiver.await.unwrap_or(TerminalEnding::Closed)
        }
        .boxed()
    }
}

impl Drop for RunningTerminal {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        // Dropping a terminal ends it, as a close without grace does.
        let request = Request::End {
            reason: None,
            grace: Duration::ZERO,
            ended: None,
        };
        if let Err(error) = self.requests.try_send(request) {
            tracing::debug!(%error, "a dropped terminal's end was not queued");
        }
    }
}

/// What happened next to a terminal.
enum Event {
    /// A chunk of a shell's output, or `None` when its console closed.
    Chunk(Option<Vec<u8>>),
    /// The shell exited on its own.
    Exited,
    /// An item of a host's output, or `None` when it ended without one.
    Item(Option<TerminalOutput>),
    /// A request of the plugin or the core, or `None` when every sender
    /// dropped.
    Request(Option<Request>),
}

/// Runs a terminal until it ends on its own or a request ends it: hands its
/// output to the plugin and does what the plugin asks, in order. Returns how
/// it ended.
async fn serve(
    mut backend: Backend,
    mut requests: mpsc::Receiver<Request>,
    output: &mpsc::Sender<TerminalOutput>,
) -> TerminalEnding {
    loop {
        let event = match &mut backend {
            Backend::Local {
                process: Some(process),
                output: chunks,
                ..
            } => {
                tokio::select! {
                    chunk = chunks.recv() => Event::Chunk(chunk),
                    _ = process.finished() => Event::Exited,
                    request = requests.recv() => Event::Request(request),
                }
            }
            Backend::Offered {
                connection: Some(_),
                output: items,
            } => {
                tokio::select! {
                    item = items.next() => Event::Item(item),
                    request = requests.recv() => Event::Request(request),
                }
            }
            Backend::Local { process: None, .. }
            | Backend::Offered {
                connection: None, ..
            } => {
                return TerminalEnding::Closed;
            }
        };
        match (&mut backend, event) {
            (_, Event::Chunk(Some(bytes)) | Event::Item(Some(TerminalOutput::Bytes { bytes }))) => {
                forward(output, bytes).await;
            }
            (
                Backend::Local {
                    process,
                    output: chunks,
                    ..
                },
                Event::Chunk(None) | Event::Exited,
            ) => {
                // The shell exited, or its console closed. What the shell
                // started ends with it (ADR-0014, decision 8), and the
                // console's last output comes out as it closes.
                let code = match process.take() {
                    Some(mut process) => {
                        let code = tokio::time::timeout(DRAIN_GRACE, process.finished())
                            .await
                            .ok()
                            .flatten();
                        process.end(Duration::ZERO).await;
                        code
                    }
                    None => None,
                };
                drain_local(chunks, output).await;
                return code.map_or(TerminalEnding::Closed, |code| TerminalEnding::Exited {
                    code,
                });
            }
            (_, Event::Item(Some(TerminalOutput::Ended { ending }))) => return ending,
            (_, Event::Item(_)) => {
                return TerminalEnding::Lost {
                    code: "connection_lost".to_owned(),
                };
            }
            (backend, Event::Request(Some(Request::Write(bytes, done)))) => {
                let _told = done.send(write_to(backend, bytes).await);
            }
            (backend, Event::Request(Some(Request::Resize(size, done)))) => {
                let _told = done.send(resize(backend, size).await);
            }
            (
                backend,
                Event::Request(Some(Request::End {
                    reason,
                    grace,
                    ended,
                })),
            ) => {
                let closed = end_backend(backend, grace, output).await;
                let ending = reason.unwrap_or(closed);
                finish_requests(&mut requests, ended, &ending).await;
                return ending;
            }
            (backend, Event::Request(None)) => {
                end_backend(backend, Duration::ZERO, output).await;
                return TerminalEnding::Closed;
            }
            (Backend::Offered { .. }, Event::Chunk(_) | Event::Exited) => {}
        }
    }
}

/// Writes the plugin's bytes to the terminal.
async fn write_to(backend: &mut Backend, bytes: Vec<u8>) -> Result<(), TerminalIoError> {
    match backend {
        Backend::Local { input, .. } => input
            .write(bytes)
            .await
            .map_err(|_error| TerminalIoError::Ended),
        Backend::Offered {
            connection: Some(connection),
            ..
        } => connection.write(bytes).await,
        Backend::Offered {
            connection: None, ..
        } => Err(TerminalIoError::Ended),
    }
}

/// Changes the terminal's size.
async fn resize(backend: &mut Backend, size: TerminalSize) -> Result<(), TerminalIoError> {
    match backend {
        Backend::Local {
            process: Some(process),
            ..
        } => process
            .resize(size.columns, size.rows)
            .map_err(|_error| TerminalIoError::Ended),
        Backend::Offered {
            connection: Some(connection),
            ..
        } => connection.resize(size).await,
        Backend::Local { process: None, .. }
        | Backend::Offered {
            connection: None, ..
        } => Err(TerminalIoError::Ended),
    }
}

/// Ends the terminal's shell or connection, and hands on the last output of
/// a shell's console. Returns how it ended.
async fn end_backend(
    backend: &mut Backend,
    grace: Duration,
    output: &mpsc::Sender<TerminalOutput>,
) -> TerminalEnding {
    match backend {
        Backend::Local {
            process,
            output: chunks,
            ..
        } => {
            let exited = match process.take() {
                Some(process) => process.end(grace).await,
                None => None,
            };
            drain_local(chunks, output).await;
            exited.map_or(TerminalEnding::Closed, |code| TerminalEnding::Exited {
                code,
            })
        }
        Backend::Offered { connection, .. } => match connection.take() {
            Some(connection) => connection.close(grace).await,
            None => TerminalEnding::Closed,
        },
    }
}

/// Hands a chunk to the plugin; a plugin that stopped reading gets nothing.
async fn forward(output: &mpsc::Sender<TerminalOutput>, bytes: Vec<u8>) {
    let _sent = output.send(TerminalOutput::Bytes { bytes }).await;
}

/// Hands the plugin what the pseudo console still wrote as it closed.
async fn drain_local(chunks: &mut mpsc::Receiver<Vec<u8>>, output: &mpsc::Sender<TerminalOutput>) {
    let drained = tokio::time::timeout(DRAIN_GRACE, async {
        while let Some(bytes) = chunks.recv().await {
            forward(output, bytes).await;
        }
    })
    .await;
    if drained.is_err() {
        tracing::debug!("a terminal's last output did not end in time");
    }
}

/// Tells the request that ended the terminal, and every request that waits,
/// how it ended.
async fn finish_requests(
    requests: &mut mpsc::Receiver<Request>,
    ended: Option<oneshot::Sender<TerminalEnding>>,
    ending: &TerminalEnding,
) {
    if let Some(ended) = ended {
        let _told = ended.send(ending.clone());
    }
    requests.close();
    while let Some(request) = requests.recv().await {
        match request {
            Request::Write(_, done) | Request::Resize(_, done) => {
                let _told = done.send(Err(TerminalIoError::Ended));
            }
            Request::End { ended, .. } => {
                if let Some(ended) = ended {
                    let _told = ended.send(ending.clone());
                }
            }
        }
    }
}
