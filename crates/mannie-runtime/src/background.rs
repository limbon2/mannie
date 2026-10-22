//! The background commands of this machine (ADR-0041, docs/features/
//! system-tools/background-commands.md): the registry that the tools
//! `start_command`, `read_command_output` and `stop_command` reach through the
//! port of `mannie-tools`, and that the owner's Stop, the end of a task, the
//! deletion of a conversation and Mannie's end stop.
//!
//! A command's record is stored through its conversation's hub with full
//! durability before its start call completes, and its end, with the archive
//! of its log, once its tree ended, so their events keep the conversation's
//! order. While a command runs, its lines stay in memory; a command that
//! ended is read from its archive.
//!
//! A plugin's tool hands the registry a command that it started on a remote
//! host through the service `background-commands` (decision 11), which
//! `remote.rs` provides.

mod notes;
mod output;
mod reads;
mod remote;

pub use output::CommandOutput;
pub(crate) use remote::BackgroundCommandsProvider;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_store::{Change, Durability, LOCAL_SOURCE, NewArchive, RuntimeStoreError};
use mannie_tools::{
    BackgroundCommands, BackgroundProcess, CommandRead, CommandStop, LiveLog, ReadRequest,
    StartRequest, Started, StopSignal, spawn_failure,
};
use mannie_types::clock::UtcMillis;
use mannie_types::ids::{ConversationId, PartId};
use mannie_types::tools::{
    BackgroundCommandDocument, BackgroundEnding, BackgroundLimit, BackgroundSource,
    MAX_COMMANDS_PER_CONVERSATION, MAX_COMMANDS_PER_MACHINE, ProcessRecord, ToolFailure,
};
use tokio::sync::{oneshot, watch};

use crate::{Runtime, lock};

/// The name of the archive of a command's log, under its start call.
const OUTPUT_ARCHIVE: &str = "output";

/// How long Mannie's end waits for every command's tree to end and its end
/// to be stored. A command left after it is interrupted at the next start.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// How the registry ends a command, and whether its end is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    /// The end is stored with this ending.
    Record(BackgroundEnding),
    /// The command's conversation was deleted, and its record with it.
    Forget,
}

/// A command that runs, as the registry follows it.
#[derive(Debug)]
struct Tracked {
    conversation: ConversationId,
    number: u32,
    part_id: PartId,
    /// The plugin whose tool started the command on a remote host, or
    /// `None` for a command of this machine.
    plugin: Option<String>,
    log: LiveLog,
    /// The command's record as stored. Each change of the record is stored
    /// while this lock is held, so the store receives them in their order.
    record: tokio::sync::Mutex<BackgroundCommandDocument>,
    /// Tells the follower to end the command; the first stop takes it.
    stop: Mutex<Option<oneshot::Sender<Stop>>>,
    /// True once the command's end is recorded.
    ended: watch::Sender<bool>,
}

impl Tracked {
    /// Asks the follower to end the command. A command that is ending
    /// already keeps the end it has.
    fn request_stop(&self, stop: Stop) {
        let sender = lock(&self.stop).take();
        if let Some(sender) = sender {
            // A follower that ended on its own has nobody to receive it.
            let _requested = sender.send(stop);
        }
    }

    /// Waits until the command's end is recorded.
    async fn wait_ended(&self) {
        let mut ended = self.ended.subscribe();
        // The sender lives as long as `self`, so the wait ends only when the
        // end is recorded.
        let _recorded = ended.wait_for(|ended| *ended).await;
    }
}

/// The commands that run on this machine.
#[derive(Debug)]
pub(crate) struct CommandRegistry {
    state: Mutex<RegistryState>,
    /// How many starts are under way, which Mannie's end and the deletion of
    /// a conversation wait for, so no command outlives them.
    starts: watch::Sender<usize>,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self {
            state: Mutex::default(),
            starts: watch::channel(0).0,
        }
    }
}

#[derive(Debug, Default)]
struct RegistryState {
    /// The commands that run, by conversation and number.
    running: HashMap<ConversationId, BTreeMap<u32, Arc<Tracked>>>,
    /// How many starts are under way in each conversation. Each holds a
    /// place under the limits until its command runs or its start failed.
    starting: HashMap<ConversationId, usize>,
    /// The conversations whose commands are being forgotten, where no
    /// command starts.
    closed_conversations: HashSet<ConversationId>,
    /// The plugins that go off, whose tools start no command.
    closed_plugins: HashSet<String>,
    /// The tickets of the plugins' calls that execute now, by their text.
    tickets: HashMap<String, remote::Ticket>,
    /// Whether Mannie quits, so no command starts.
    closed: bool,
}

/// Why the registry refused a start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// Mannie quits, or the conversation is being deleted.
    Closed,
    /// As many commands run as the limit allows.
    TooMany(BackgroundLimit, u32),
}

impl RegistryState {
    /// Whether one more command may start in the conversation.
    fn admits(&self, conversation: &ConversationId) -> Result<(), Refusal> {
        if self.closed || self.closed_conversations.contains(conversation) {
            return Err(Refusal::Closed);
        }
        if self.count_in(conversation) >= limit(MAX_COMMANDS_PER_CONVERSATION) {
            return Err(Refusal::TooMany(
                BackgroundLimit::Conversation,
                MAX_COMMANDS_PER_CONVERSATION,
            ));
        }
        if self.count() >= limit(MAX_COMMANDS_PER_MACHINE) {
            return Err(Refusal::TooMany(
                BackgroundLimit::Machine,
                MAX_COMMANDS_PER_MACHINE,
            ));
        }
        Ok(())
    }

    fn count_in(&self, conversation: &ConversationId) -> usize {
        let running = self.running.get(conversation).map_or(0, BTreeMap::len);
        running + self.starting.get(conversation).copied().unwrap_or(0)
    }

    fn count(&self) -> usize {
        let running: usize = self.running.values().map(BTreeMap::len).sum();
        running + self.starting.values().sum::<usize>()
    }
}

/// A place under the limits that a start holds until its command is
/// registered or its start failed.
struct Place<'a> {
    registry: &'a CommandRegistry,
    conversation: ConversationId,
}

impl Place<'_> {
    /// Registers the started command, which takes the place's own.
    fn register(self, tracked: Arc<Tracked>) {
        lock(&self.registry.state)
            .running
            .entry(self.conversation.clone())
            .or_default()
            .insert(tracked.number, tracked);
    }
}

impl Drop for Place<'_> {
    fn drop(&mut self) {
        {
            let mut state = lock(&self.registry.state);
            if let Some(count) = state.starting.get_mut(&self.conversation) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    state.starting.remove(&self.conversation);
                }
            }
        }
        self.registry
            .starts
            .send_modify(|count| *count = count.saturating_sub(1));
    }
}

fn limit(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

impl CommandRegistry {
    /// Takes a place for a start in a conversation, within the limits of the
    /// conversation and of the machine.
    fn reserve(&self, conversation: &ConversationId) -> Result<Place<'_>, ToolFailure> {
        let state = lock(&self.state);
        state
            .admits(conversation)
            .map_err(|refusal| match refusal {
                Refusal::Closed => ToolFailure::IoError {
                    kind: "no background command starts while Mannie quits or the conversation \
                           is deleted"
                        .to_owned(),
                },
                Refusal::TooMany(scope, limit) => {
                    ToolFailure::TooManyBackgroundCommands { scope, limit }
                }
            })?;
        Ok(self.hold(state, conversation))
    }

    /// Takes a place for a start that the state admitted, before its lock is
    /// let go.
    fn hold(
        &self,
        mut state: MutexGuard<'_, RegistryState>,
        conversation: &ConversationId,
    ) -> Place<'_> {
        *state.starting.entry(conversation.clone()).or_default() += 1;
        drop(state);
        self.starts.send_modify(|count| *count += 1);
        Place {
            registry: self,
            conversation: conversation.clone(),
        }
    }

    fn tracked(&self, conversation: &ConversationId, number: u32) -> Option<Arc<Tracked>> {
        lock(&self.state)
            .running
            .get(conversation)
            .and_then(|commands| commands.get(&number))
            .map(Arc::clone)
    }

    fn running_in(&self, conversation: &ConversationId) -> Vec<Arc<Tracked>> {
        lock(&self.state)
            .running
            .get(conversation)
            .map(|commands| commands.values().map(Arc::clone).collect())
            .unwrap_or_default()
    }

    fn all(&self) -> Vec<Arc<Tracked>> {
        lock(&self.state)
            .running
            .values()
            .flat_map(|commands| commands.values().map(Arc::clone))
            .collect()
    }

    /// The highest number of a conversation's command that runs, or 0.
    fn highest(&self, conversation: &ConversationId) -> u32 {
        lock(&self.state)
            .running
            .get(conversation)
            .and_then(|commands| commands.keys().next_back().copied())
            .unwrap_or(0)
    }

    fn remove(&self, tracked: &Tracked) {
        let mut state = lock(&self.state);
        if let Some(commands) = state.running.get_mut(&tracked.conversation) {
            commands.remove(&tracked.number);
            if commands.is_empty() {
                state.running.remove(&tracked.conversation);
            }
        }
    }

    /// How many commands run.
    fn running_count(&self) -> usize {
        lock(&self.state).running.values().map(BTreeMap::len).sum()
    }

    /// The commands that a plugin's tools started and that run.
    fn of_plugin(&self, plugin: &str) -> Vec<Arc<Tracked>> {
        lock(&self.state)
            .running
            .values()
            .flat_map(BTreeMap::values)
            .filter(|tracked| tracked.plugin.as_deref() == Some(plugin))
            .map(Arc::clone)
            .collect()
    }

    /// Refuses every start from now on.
    fn close(&self) {
        lock(&self.state).closed = true;
    }

    /// Refuses the starts of a conversation until [`Self::reopen`].
    fn close_conversation(&self, conversation: &ConversationId) {
        lock(&self.state)
            .closed_conversations
            .insert(conversation.clone());
    }

    fn reopen(&self, conversation: &ConversationId) {
        lock(&self.state).closed_conversations.remove(conversation);
    }

    /// Waits until no start is under way in the conversation, or anywhere
    /// for `None`, so the commands that they start are registered.
    async fn starts_done(&self, conversation: Option<&ConversationId>) {
        let mut starts = self.starts.subscribe();
        let _done = starts
            .wait_for(|count| match conversation {
                None => *count == 0,
                Some(conversation) => !lock(&self.state).starting.contains_key(conversation),
            })
            .await;
    }
}

/// The time between two moments of the injected clock.
fn between(start: UtcMillis, end: UtcMillis) -> Duration {
    let millis = end.as_millis().saturating_sub(start.as_millis());
    Duration::from_millis(u64::try_from(millis).unwrap_or(0))
}

/// How long a command ran, or has run so far.
fn ran_for(document: &BackgroundCommandDocument, now: UtcMillis) -> Duration {
    between(document.started_at, document.ended_at.unwrap_or(now))
}

/// What a command's end reported beside how it ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Reported {
    /// Its exit code, when it ended with one.
    exit_code: Option<i32>,
    /// The signal that its host reported, when it reported one.
    signal: Option<String>,
}

/// The record of a command that just started, named by its start call's
/// activity.
fn started_document(
    command: String,
    activity: Option<String>,
    source: BackgroundSource,
    process: Option<ProcessRecord>,
    started_at: UtcMillis,
) -> BackgroundCommandDocument {
    BackgroundCommandDocument {
        command,
        activity,
        source,
        process,
        started_at,
        ending: None,
        exit_code: None,
        signal: None,
        ended_at: None,
        lines: 0,
        bytes: 0,
        replaced: false,
        head_lines: 0,
        left_out: None,
        archive_id: None,
        read_up_to: 0,
        told: false,
    }
}

/// A command whose record is stored, as the registry takes it.
struct Start {
    conversation: ConversationId,
    number: u32,
    part_id: PartId,
    plugin: Option<String>,
    log: LiveLog,
    document: BackgroundCommandDocument,
}

impl Start {
    /// Registers the command in the place that its start held, and returns
    /// it with the stops that its follower receives.
    fn register(self, place: Place<'_>) -> (Arc<Tracked>, oneshot::Receiver<Stop>) {
        tracing::debug!(conversation = %self.conversation, number = self.number, command = %self.document.command, "the text of a background command");
        let (stop, stops) = oneshot::channel();
        let tracked = Arc::new(Tracked {
            conversation: self.conversation,
            number: self.number,
            part_id: self.part_id,
            plugin: self.plugin,
            log: self.log,
            record: tokio::sync::Mutex::new(self.document),
            stop: Mutex::new(Some(stop)),
            ended: watch::channel(false).0,
        });
        place.register(Arc::clone(&tracked));
        tracing::info!(conversation = %tracked.conversation, number = tracked.number, plugin = tracked.plugin.as_deref(), "a background command started");
        (tracked, stops)
    }
}

/// A failure of the store as the tools report it.
fn store_failure(error: &RuntimeStoreError, what: &str) -> ToolFailure {
    tracing::error!(%error, error.code = error.code(), "could not {what}");
    ToolFailure::IoError {
        kind: format!("could not {what}"),
    }
}

/// What `stop_command` and the owner's Stop return about a command.
fn command_stop(
    document: &BackgroundCommandDocument,
    number: u32,
    part_id: PartId,
    now: UtcMillis,
    had_ended: bool,
) -> CommandStop {
    CommandStop {
        view: document.view(number, part_id),
        ran_for: ran_for(document, now),
        had_ended,
        unread: document.lines.saturating_sub(document.read_up_to),
        remote: matches!(document.source, BackgroundSource::Remote { .. }),
    }
}

/// The registry as the tools reach it, through the port of `mannie-tools`.
#[derive(Debug)]
pub(crate) struct BackgroundPort {
    pub(crate) runtime: Runtime,
}

/// Runs a call's work on the registry in a task of its own. The work stores
/// through the conversation's hub, and the run that waits for the call
/// stores its journal through the same hub without polling the call
/// meanwhile, so the call's future must not hold the hub while it waits.
fn spawned<T: Send + 'static>(
    work: impl Future<Output = Result<T, ToolFailure>> + Send + 'static,
) -> BoxFuture<'static, Result<T, ToolFailure>> {
    let task = tokio::spawn(work);
    async move {
        task.await.unwrap_or_else(|error| {
            Err(ToolFailure::IoError {
                kind: format!("the work on a background command stopped unexpectedly: {error}"),
            })
        })
    }
    .boxed()
}

impl BackgroundCommands for BackgroundPort {
    fn start(&self, request: StartRequest) -> BoxFuture<'static, Result<Started, ToolFailure>> {
        let runtime = self.runtime.clone();
        spawned(async move { runtime.start_background(request).await })
    }

    fn read(
        &self,
        request: ReadRequest,
        stop: StopSignal,
    ) -> BoxFuture<'static, Result<CommandRead, ToolFailure>> {
        let runtime = self.runtime.clone();
        spawned(async move { runtime.read_background(&request, &stop).await })
    }

    fn stop(
        &self,
        conversation: ConversationId,
        number: u32,
    ) -> BoxFuture<'static, Result<CommandStop, ToolFailure>> {
        let runtime = self.runtime.clone();
        spawned(async move {
            runtime
                .stop_background(&conversation, number, BackgroundEnding::StoppedByAgent)
                .await
        })
    }
}

impl Runtime {
    /// Starts a command in the background: a place under the limits, its
    /// tree, its process in the call's journal, and its record, stored with
    /// full durability before the call completes. A record that cannot be
    /// stored ends the tree.
    async fn start_background(&self, request: StartRequest) -> Result<Started, ToolFailure> {
        let registry = &self.inner.background;
        let place = registry.reserve(&request.conversation)?;
        let StartRequest {
            conversation,
            part_id,
            activity,
            spec,
            process: journal,
        } = request;
        let number = self
            .next_background_number(&conversation)
            .await
            .map_err(|error| store_failure(&error, "read the numbers of background commands"))?;
        let process = BackgroundProcess::spawn(&spec)
            .await
            .map_err(|error| spawn_failure(&error, &spec.shell))?;
        let record = process.record().clone();
        if let Some(journal) = journal {
            // A call whose run went away has no journal left to write.
            let _journaled = journal.send(record.clone());
        }
        let document = started_document(
            spec.text.clone(),
            activity,
            BackgroundSource::Local {
                shell: spec.shell.to_string_lossy().into_owned(),
                working_directory: spec.working_directory.to_string_lossy().into_owned(),
            },
            Some(record),
            self.inner.clock.now(),
        );
        let stored = self
            .store_start(&conversation, number, &part_id, LOCAL_SOURCE, &document)
            .await;
        if let Err(error) = stored {
            // Nothing would know of a command without its record, so it ends
            // before the call fails.
            process.end(false).await;
            return Err(store_failure(
                &error,
                "store the record of a background command, so the command was ended",
            ));
        }
        let start = Start {
            conversation,
            number,
            part_id,
            plugin: None,
            log: process.log().clone(),
            document,
        };
        let (tracked, stops) = start.register(place);
        tokio::spawn(self.clone().follow(tracked, process, stops));
        Ok(Started { number })
    }

    /// The number of a conversation's next command: past every stored
    /// command, and past those that run, whose records may be stored after a
    /// deletion of the conversation's records began.
    async fn next_background_number(
        &self,
        conversation: &ConversationId,
    ) -> Result<u32, RuntimeStoreError> {
        let stored_next = self
            .inner
            .store
            .next_background_number(conversation)
            .await?;
        let running_next = self
            .inner
            .background
            .highest(conversation)
            .saturating_add(1);
        Ok(stored_next.max(running_next))
    }

    /// Stores the record of a command that started, with full durability,
    /// through its conversation's hub.
    async fn store_start(
        &self,
        conversation: &ConversationId,
        number: u32,
        part_id: &PartId,
        source: &str,
        document: &BackgroundCommandDocument,
    ) -> Result<(), RuntimeStoreError> {
        let change = Change::StartBackgroundCommand {
            number,
            part_id: part_id.clone(),
            source: source.to_owned(),
            document: Box::new(document.clone()),
        };
        self.inner
            .hubs
            .apply(
                &self.inner.store,
                conversation,
                vec![change],
                Durability::Full,
            )
            .await
            .map(|_applied| ())
    }

    /// Follows a command until it ends on its own or a stop ends it, ends
    /// what is left of its tree, and records its end.
    async fn follow(
        self,
        tracked: Arc<Tracked>,
        mut process: BackgroundProcess,
        mut stops: oneshot::Receiver<Stop>,
    ) {
        let (stop, exit_code) = tokio::select! {
            exit_code = process.finished() => (Stop::Record(BackgroundEnding::Exited), exit_code),
            // A registry that went away is Mannie's end.
            stop = &mut stops => (stop.unwrap_or(Stop::Record(BackgroundEnding::Interrupted)), None),
        };
        // After its own end, the shell exits and what it left ends; a stop
        // ends the tree, then the shell (ADR-0014, decision 8).
        let own_end = stop == Stop::Record(BackgroundEnding::Exited);
        process.end(own_end).await;
        let reported = Reported {
            exit_code,
            signal: None,
        };
        self.record_end(&tracked, stop, reported).await;
    }

    /// Records a command's end: its ending, its totals and the archive of
    /// its log under its start call, with an event, unless its conversation
    /// was deleted. Then it leaves the registry, and its waiters go on.
    async fn record_end(&self, tracked: &Tracked, stop: Stop, reported: Reported) {
        let mut record = tracked.record.lock().await;
        match stop {
            Stop::Record(ending) => {
                let archived = tracked.log.archive();
                let archive_id = self.inner.ids.archive();
                record.ending = Some(ending);
                record.exit_code = reported.exit_code;
                record.signal = reported.signal;
                record.ended_at = Some(self.inner.clock.now());
                record.lines = archived.lines;
                record.bytes = archived.bytes;
                record.replaced = archived.replaced;
                record.head_lines = archived.head_lines;
                record.left_out = archived.left_out;
                record.archive_id = Some(archive_id.clone());
                // The agent's own stop is told by its call's result.
                record.told = !ending.is_news();
                let change = Change::UpdateBackgroundCommand {
                    number: tracked.number,
                    document: Box::new(record.clone()),
                    archive: Some(NewArchive {
                        id: archive_id,
                        name: OUTPUT_ARCHIVE.to_owned(),
                        content: archived.text,
                    }),
                    announce: true,
                };
                let stored = self
                    .inner
                    .hubs
                    .apply(
                        &self.inner.store,
                        &tracked.conversation,
                        vec![change],
                        Durability::Full,
                    )
                    .await;
                match stored {
                    Ok(_) | Err(RuntimeStoreError::ConversationNotFound) => {}
                    Err(error) => {
                        tracing::error!(%error, error.code = error.code(), conversation = %tracked.conversation, number = tracked.number, "could not store the end of a background command; the next start marks it as interrupted");
                    }
                }
                tracing::info!(conversation = %tracked.conversation, number = tracked.number, ending = ending.as_str(), exit_code = record.exit_code, "a background command ended");
            }
            Stop::Forget => {
                // Its record went with its conversation; the ending only
                // tells the waiters that it ended.
                record.ending = Some(BackgroundEnding::StoppedByOwner);
                record.ended_at = Some(self.inner.clock.now());
                tracing::info!(conversation = %tracked.conversation, number = tracked.number, "a background command of a deleted conversation ended");
            }
        }
        drop(record);
        self.inner.background.remove(tracked);
        tracked.ended.send_replace(true);
        if let Stop::Record(ending) = stop
            && ending.is_its_own()
        {
            self.command_ended_on_its_own(&tracked.conversation);
        }
    }

    /// Stops a command and waits until its end is recorded. A command that
    /// ended already is left as it is, and says how it ended.
    pub(crate) async fn stop_background(
        &self,
        conversation: &ConversationId,
        number: u32,
        ending: BackgroundEnding,
    ) -> Result<CommandStop, ToolFailure> {
        let now = self.inner.clock.now();
        let Some(tracked) = self.inner.background.tracked(conversation, number) else {
            let stored = self
                .inner
                .store
                .background_command(conversation, number)
                .await
                .map_err(|error| store_failure(&error, "read a background command"))?
                .ok_or(ToolFailure::CommandNotFound { number })?;
            return Ok(command_stop(
                &stored.document,
                number,
                stored.part_id,
                now,
                true,
            ));
        };
        tracing::info!(conversation = %conversation, number, ending = ending.as_str(), "stopping a background command");
        tracked.request_stop(Stop::Record(ending));
        tracked.wait_ended().await;
        let mut record = tracked.record.lock().await;
        let had_ended = record.ending != Some(ending);
        // The agent's call tells an end that came first, too.
        if ending == BackgroundEnding::StoppedByAgent && !record.told {
            record.told = true;
            self.store_record(&tracked, &record).await;
        }
        Ok(command_stop(
            &record,
            number,
            tracked.part_id.clone(),
            self.inner.clock.now(),
            had_ended,
        ))
    }

    /// Stores a change of a command's record that no view shows.
    async fn store_record(&self, tracked: &Tracked, record: &BackgroundCommandDocument) {
        let change = Change::UpdateBackgroundCommand {
            number: tracked.number,
            document: Box::new(record.clone()),
            archive: None,
            announce: false,
        };
        let stored = self
            .inner
            .hubs
            .apply(
                &self.inner.store,
                &tracked.conversation,
                vec![change],
                Durability::Normal,
            )
            .await;
        if let Err(error) = stored {
            tracing::warn!(%error, error.code = error.code(), conversation = %tracked.conversation, number = tracked.number, "could not store the read position of a background command");
        }
    }

    /// Ends the commands of a task conversation whose task's run ended, and
    /// waits until their ends are stored, before the task's ending
    /// (ADR-0041, decision 9).
    pub(crate) async fn end_task_commands(&self, conversation: &ConversationId) {
        let commands = self.inner.background.running_in(conversation);
        if commands.is_empty() {
            return;
        }
        tracing::info!(conversation = %conversation, commands = commands.len(), "ending the background commands of a task");
        for command in &commands {
            command.request_stop(Stop::Record(BackgroundEnding::TaskEnded));
        }
        futures_util::future::join_all(commands.iter().map(|command| command.wait_ended())).await;
    }

    /// Ends the commands of deleted conversations without storing their
    /// ends, since their records went with the conversations. A start under
    /// way in one of them is waited for, so no command outlives its
    /// conversation.
    pub(crate) async fn forget_background_commands(&self, conversations: &[ConversationId]) {
        let registry = &self.inner.background;
        for conversation in conversations {
            registry.close_conversation(conversation);
            registry.starts_done(Some(conversation)).await;
            let commands = registry.running_in(conversation);
            for command in &commands {
                command.request_stop(Stop::Forget);
            }
            futures_util::future::join_all(commands.iter().map(|command| command.wait_ended()))
                .await;
            registry.reopen(conversation);
            self.inner.wake_ups.forget(conversation);
        }
    }

    /// Ends every command as interrupted, at Mannie's end: no command starts
    /// any more, the starts under way finish first, and every tree ends at
    /// the same time before the ends and archives are stored. A shell is
    /// never killed before its tree, which only a walk made while it lives
    /// reaches on Unix.
    pub(crate) async fn end_background_commands(&self) {
        let registry = &self.inner.background;
        registry.close();
        let ended = async {
            registry.starts_done(None).await;
            let commands = registry.all();
            if !commands.is_empty() {
                tracing::info!(commands = commands.len(), "ending the background commands");
            }
            for command in &commands {
                command.request_stop(Stop::Record(BackgroundEnding::Interrupted));
            }
            futures_util::future::join_all(commands.iter().map(|command| command.wait_ended()))
                .await;
        };
        if tokio::time::timeout(SHUTDOWN_TIMEOUT, ended).await.is_err() {
            tracing::warn!(
                "background commands did not end in time; the next start marks them as interrupted"
            );
        }
    }

    /// How many background commands run on this machine, which the quit
    /// warning names.
    pub fn running_background_count(&self) -> u32 {
        u32::try_from(self.inner.background.running_count()).unwrap_or(u32::MAX)
    }
}
