//! What notes tell the model of background commands (docs/features/
//! system-tools/wake-ups.md, notes): the ends that it was not told, and at
//! the start of a reply that the owner's message starts, the commands that
//! run. A note names commands and never holds their output.

use mannie_store::RuntimeStoreError;
use mannie_types::clock::UtcMillis;
use mannie_types::ids::ConversationId;
use mannie_types::tools::{BackgroundCommandDocument, BackgroundSource};

use super::ran_for;
use crate::Runtime;
use crate::texts::{self, NotedCommand};

/// The note that joins a request, and the ends that it tells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpeningNote {
    /// The note.
    pub(crate) text: String,
    /// The numbers of the commands whose ends it tells.
    pub(crate) told: Vec<u32>,
}

/// A command as a note names it.
fn noted(
    number: u32,
    document: &BackgroundCommandDocument,
    lines: u64,
    now: UtcMillis,
) -> NotedCommand {
    NotedCommand {
        number,
        command: document.command.clone(),
        ending: document.ending,
        exit_code: document.exit_code,
        signal: document.signal.clone(),
        ran_for: ran_for(document, now),
        unread: lines.saturating_sub(document.read_up_to),
        remote: matches!(document.source, BackgroundSource::Remote { .. }),
    }
}

impl Runtime {
    /// The ends of a conversation's background commands that the model was
    /// not told, oldest first.
    pub(crate) async fn untold_ends(
        &self,
        conversation: &ConversationId,
    ) -> Result<Vec<NotedCommand>, RuntimeStoreError> {
        let now = self.inner.clock.now();
        let commands = self.inner.store.background_commands(conversation).await?;
        Ok(commands
            .iter()
            .filter(|command| command.document.ending.is_some() && !command.document.told)
            .map(|command| {
                noted(
                    command.number,
                    &command.document,
                    command.document.lines,
                    now,
                )
            })
            .collect())
    }

    /// The note that starts a reply of the owner's message, when the
    /// conversation has commands that run or ends that the model was not
    /// told.
    pub(crate) async fn opening_note(
        &self,
        conversation: &ConversationId,
    ) -> Result<Option<OpeningNote>, RuntimeStoreError> {
        let now = self.inner.clock.now();
        let mut running = Vec::new();
        for tracked in self.inner.background.running_in(conversation) {
            let record = tracked.record.lock().await;
            running.push(noted(
                tracked.number,
                &record,
                tracked.log.mark().lines,
                now,
            ));
        }
        // Read after the commands that run, so a command that ends between
        // the two reads is named once, by its stored end.
        let ended = self.untold_ends(conversation).await?;
        if ended.is_empty() && running.is_empty() {
            return Ok(None);
        }
        let told: Vec<u32> = ended.iter().map(|command| command.number).collect();
        running.retain(|command| !told.contains(&command.number));
        let mut commands = ended;
        commands.extend(running);
        commands.sort_by_key(|command| command.number);
        Ok(Some(OpeningNote {
            text: texts::background_note(&commands),
            told,
        }))
    }
}
