//! Wake-ups (ADR-0041, docs/features/system-tools/wake-ups.md, waking the
//! agent): when a background command of one of the owner's conversations ends
//! on its own while no reply runs there, Mannie starts a reply whose request
//! is its note of the ends. Limits bound what runs while the owner is away:
//! ends that come together share a wake-up, a conversation wakes at most six
//! times an hour, the owner's Stop pauses it until their next message, and
//! the Tools setting turns wake-ups off.
//!
//! Mannie checks when it records such an end, after the delay that gathers
//! ends, and again whenever the conversation becomes free. The check reserves
//! the conversation as the owner's message does, so a wake-up and the
//! owner's message never both start a reply, and the turn of the wake-up
//! marks the ends that it tells in its own transaction.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use mannie_store::{Change, Durability};
use mannie_types::chat::ChatError;
use mannie_types::clock::UtcMillis;
use mannie_types::ids::ConversationId;
use mannie_types::tools::{
    BackgroundEnding, CommandEndView, CommandNoteView, MAX_VIEW_COMMAND_CHARS, ToolsSettings,
};

use crate::conversations::store_failure;
use crate::texts::{self, NotedCommand};
use crate::{Busy, Runtime, lock};

/// How many times a conversation wakes in an hour.
const MAX_WAKE_UPS_PER_HOUR: usize = 6;

/// The window of that limit.
const HOUR: Duration = Duration::from_hours(1);

/// What wake-ups know of the conversations.
#[derive(Debug, Default)]
pub(crate) struct WakeUps {
    state: Mutex<WakeState>,
}

#[derive(Debug, Default)]
struct WakeState {
    /// Conversations with an end of its own that may wake them once they
    /// are free.
    due: HashSet<ConversationId>,
    /// Conversations whose ends wait for the delay that lets ends that come
    /// together share one wake-up.
    gathering: HashSet<ConversationId>,
    /// The times of each conversation's wake-ups within the last hour.
    recent: HashMap<ConversationId, VecDeque<UtcMillis>>,
    /// Conversations whose owner stopped a reply, which do not wake until
    /// the owner's next message.
    paused: HashSet<ConversationId>,
}

/// What a check did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Woke {
    /// A reply started.
    Started,
    /// The conversation is busy; it checks again once it is free.
    Busy,
    /// Nothing wakes it now: no end of its own is untold, or a limit, the
    /// pause, the setting or Mannie's end holds it. Its ends stay untold
    /// for its next reply.
    Held,
}

fn between(start: UtcMillis, end: UtcMillis) -> Duration {
    let millis = end.as_millis().saturating_sub(start.as_millis());
    Duration::from_millis(u64::try_from(millis).unwrap_or(0))
}

/// An end as a wake-up's row names it.
fn end_view(command: &NotedCommand) -> Option<CommandEndView> {
    Some(CommandEndView {
        number: command.number,
        command: command
            .command
            .chars()
            .take(MAX_VIEW_COMMAND_CHARS)
            .collect(),
        ending: command.ending?,
        exit_code: command.exit_code,
        signal: command.signal.clone(),
        unread: u32::try_from(command.unread).unwrap_or(u32::MAX),
    })
}

impl WakeUps {
    /// Marks a conversation as due, and returns whether it starts gathering
    /// ends now.
    fn due_after_end(&self, conversation: &ConversationId) -> bool {
        let mut state = lock(&self.state);
        state.due.insert(conversation.clone());
        state.gathering.insert(conversation.clone())
    }

    fn gathered(&self, conversation: &ConversationId) {
        lock(&self.state).gathering.remove(conversation);
    }

    /// Whether a conversation that became free should check now: it is due
    /// and its ends are not being gathered.
    fn check_when_free(&self, conversation: &ConversationId) -> bool {
        let state = lock(&self.state);
        state.due.contains(conversation) && !state.gathering.contains(conversation)
    }

    /// Takes a conversation's due mark, so the release of the check's own
    /// reservation does not check again.
    fn take_due(&self, conversation: &ConversationId) -> bool {
        lock(&self.state).due.remove(conversation)
    }

    fn set_due(&self, conversation: &ConversationId) {
        lock(&self.state).due.insert(conversation.clone());
    }

    fn is_paused(&self, conversation: &ConversationId) -> bool {
        lock(&self.state).paused.contains(conversation)
    }

    /// Whether the conversation woke fewer times than the limit within the
    /// hour before `now`.
    fn within_limit(&self, conversation: &ConversationId, now: UtcMillis) -> bool {
        let mut state = lock(&self.state);
        let recent = state.recent.entry(conversation.clone()).or_default();
        while recent.front().is_some_and(|at| between(*at, now) >= HOUR) {
            recent.pop_front();
        }
        recent.len() < MAX_WAKE_UPS_PER_HOUR
    }

    fn record(&self, conversation: &ConversationId, now: UtcMillis) {
        lock(&self.state)
            .recent
            .entry(conversation.clone())
            .or_default()
            .push_back(now);
    }

    /// The owner stopped a reply of the conversation.
    pub(crate) fn pause(&self, conversation: &ConversationId) {
        lock(&self.state).paused.insert(conversation.clone());
    }

    /// The owner sent the conversation a message.
    pub(crate) fn resume(&self, conversation: &ConversationId) {
        lock(&self.state).paused.remove(conversation);
    }

    /// The conversation was deleted.
    pub(crate) fn forget(&self, conversation: &ConversationId) {
        let mut state = lock(&self.state);
        state.due.remove(conversation);
        state.gathering.remove(conversation);
        state.recent.remove(conversation);
        state.paused.remove(conversation);
    }
}

impl Runtime {
    /// A background command of the conversation ended on its own: after the
    /// delay that gathers ends that come together, the conversation wakes if
    /// it is free.
    pub(crate) fn command_ended_on_its_own(&self, conversation: &ConversationId) {
        if !self.inner.wake_ups.due_after_end(conversation) {
            // The ends that are being gathered already wake it together.
            return;
        }
        let runtime = self.clone();
        let conversation = conversation.clone();
        tokio::spawn(async move {
            tokio::time::sleep(runtime.inner.config.wake_up_delay).await;
            runtime.inner.wake_ups.gathered(&conversation);
            runtime.wake_if_due(conversation).await;
        });
    }

    /// A conversation became free: a reply ended, or a command that reserved
    /// it let it go. A conversation with an end of its own that is untold
    /// checks whether it wakes.
    pub(crate) fn conversation_free(&self, conversation: &ConversationId) {
        if !self.inner.wake_ups.check_when_free(conversation) {
            return;
        }
        // A release outside a running Tokio runtime only happens while the
        // process ends, when nothing wakes.
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let runtime = self.clone();
        let conversation = conversation.clone();
        handle.spawn(async move { runtime.wake_if_due(conversation).await });
    }

    /// Wakes a conversation that is due, when nothing holds it.
    async fn wake_if_due(self, conversation: ConversationId) {
        if !self.inner.wake_ups.take_due(&conversation) {
            return;
        }
        match self.wake(&conversation).await {
            Ok(Woke::Started | Woke::Held) => {}
            Ok(Woke::Busy) => self.inner.wake_ups.set_due(&conversation),
            Err(error) => {
                tracing::warn!(conversation = %conversation, error.code = error.code(), "a wake-up could not start; the ends stay untold for the next reply");
            }
        }
    }

    /// Starts the reply of a wake-up, unless something holds the
    /// conversation.
    async fn wake(&self, conversation: &ConversationId) -> Result<Woke, ChatError> {
        if self.is_shutting_down() || self.inner.wake_ups.is_paused(conversation) {
            return Ok(Woke::Held);
        }
        let settings = self
            .inner
            .store
            .load_settings::<ToolsSettings>()
            .await
            .map_err(|error| {
                tracing::warn!(%error, error.code = error.code(), "could not read the tools settings for a wake-up");
                ChatError::StorageFailed
            })?;
        if !settings.wake_on_command_end {
            return Ok(Woke::Held);
        }
        let Ok(reservation) = self.reserve(conversation, Busy::Starting) else {
            return Ok(Woke::Busy);
        };
        let Some(summary) = self
            .inner
            .store
            .conversation_summary(conversation)
            .await
            .map_err(store_failure)?
        else {
            return Ok(Woke::Held);
        };
        // A task conversation's run answers to an agent, never to an end.
        if summary.task.is_some() {
            return Ok(Woke::Held);
        }
        let ended = self
            .untold_ends(conversation)
            .await
            .map_err(store_failure)?;
        let own_end = ended
            .iter()
            .any(|command| command.ending.is_some_and(BackgroundEnding::is_its_own));
        let now = self.inner.clock.now();
        if !own_end {
            return Ok(Woke::Held);
        }
        if !self.inner.wake_ups.within_limit(conversation, now) {
            tracing::info!(conversation = %conversation, "a conversation woke as often as an hour allows; the ends wait for the next reply");
            return Ok(Woke::Held);
        }
        let profile = self.profile_for(summary.profile_id.as_ref()).await?;
        let turn_id = self.inner.ids.turn();
        let run = self.new_run(&profile);
        let numbers: Vec<u32> = ended.iter().map(|command| command.number).collect();
        let changes = vec![
            Change::StartCommandTurn {
                turn_id: turn_id.clone(),
                part_id: self.inner.ids.part(),
                text: texts::wake_up_note(&ended),
                note: CommandNoteView {
                    ends: ended.iter().filter_map(end_view).collect(),
                },
                run: run.clone(),
            },
            Change::TellBackgroundEnds {
                numbers: numbers.clone(),
            },
        ];
        self.inner
            .hubs
            .apply(&self.inner.store, conversation, changes, Durability::Full)
            .await
            .map_err(store_failure)?;
        self.inner.wake_ups.record(conversation, now);
        tracing::info!(conversation = %conversation, commands = ?numbers, "the end of a background command woke the agent");
        self.start_reserved(reservation, &turn_id, &run);
        Ok(Woke::Started)
    }
}
