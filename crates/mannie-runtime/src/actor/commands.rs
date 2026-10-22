//! What a run tells its model of its conversation's background commands
//! (docs/features/system-tools/wake-ups.md, notes): after a step's tool
//! calls, the ends that the model was not told, and once more before a reply
//! would end with ends untold.

use mannie_store::Change;
use mannie_types::run::FinishReason;

use super::{Ending, RunActor};
use crate::calls::Setup;
use crate::texts;

/// How often a reply continues to tell the model of ends before it ends.
const MAX_END_NOTES: u32 = 1;

impl RunActor {
    /// Adds a note with the ends of background commands that the model was
    /// not told, so it reads them in its next call, and returns whether it
    /// did. The note does not count against the reply's continuation notes.
    pub(super) async fn note_command_ends(&mut self) -> Result<bool, Ending> {
        let ended = self.runtime.untold_ends(&self.conversation).await?;
        if ended.is_empty() {
            return Ok(false);
        }
        let numbers: Vec<u32> = ended.iter().map(|command| command.number).collect();
        tracing::info!(run = %self.run, commands = ?numbers, "telling the model of background commands that ended");
        // The note and the mark are stored in one batch, so an end is told
        // once.
        self.pending.push(Change::TellBackgroundEnds { numbers });
        self.add_note(&texts::background_note(&ended), false)
            .await?;
        Ok(true)
    }

    /// Before a reply ends: a reply with ends untold continues once with the
    /// note, unless its tool calls are used up, as a reply with unread
    /// reports does. An end after this wakes the agent once the reply ended.
    pub(super) async fn note_ends_before_the_end(
        &mut self,
        setup: &Setup,
        finish: FinishReason,
    ) -> Result<bool, Ending> {
        let may_continue = matches!(finish, FinishReason::Stop | FinishReason::Other);
        if self.end_notes >= MAX_END_NOTES
            || !may_continue
            || !setup.uses_tools()
            || self.tools_used_up(setup)
        {
            return Ok(false);
        }
        let noted = self.note_command_ends().await?;
        if noted {
            self.end_notes += 1;
        }
        Ok(noted)
    }
}
