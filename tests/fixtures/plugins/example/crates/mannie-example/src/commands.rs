//! The plugin's command and channel, as specs of the plugin SDK (ADR-0034,
//! decision 8), so the host checks their input before the plugin's handlers
//! run and answers `plugin_off` while the plugin is off.

use mannie_plugin_sdk::ipc::{
    ChannelSpec, CommandSpec, InputRefusal, Interface, PluginError, Validate,
};
use serde::{Deserialize, Serialize};
use specta::Type;

/// The longest note that the plugin keeps, in characters.
pub const MAX_NOTE_CHARACTERS: u32 = 1000;

/// The argument of `add_note`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AddNote {
    /// The note's text.
    pub text: String,
}

impl Validate for AddNote {
    /// The text is not blank, has at most [`MAX_NOTE_CHARACTERS`] characters
    /// and no control characters other than line breaks.
    fn validate(&self) -> Result<(), InputRefusal> {
        let is_blank = self.text.trim().is_empty();
        let is_long = self.text.chars().count() > MAX_NOTE_CHARACTERS as usize;
        let has_control = self
            .text
            .chars()
            .any(|character| character.is_control() && character != '\n');
        if is_blank || is_long || has_control {
            return Err(InputRefusal::new("text"));
        }
        Ok(())
    }
}

/// A note that the plugin keeps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    /// The note's ID.
    pub id: String,
    /// The note's text.
    pub text: String,
}

/// Why a command or channel of the plugin failed. The host's refusals, such
/// as `plugin_off`, come beside these (ADR-0034, decision 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum ExampleError {
    /// The store did not keep the note.
    #[error("the note could not be stored")]
    StorageFailed,
}

impl PluginError for ExampleError {
    /// The stable code of the error, for logs.
    fn code(&self) -> &'static str {
        match self {
            Self::StorageFailed => "example.storage_failed",
        }
    }
}

/// `add_note`: keeps a note and hands it to the channels that follow the
/// notes.
#[derive(Debug)]
pub enum AddNoteCommand {}

impl CommandSpec for AddNoteCommand {
    const NAME: &'static str = "add_note";
    const DOCS: &'static str = "Keeps a note and hands it to every channel that follows the notes.";
    type Input = AddNote;
    type Output = Note;
    type Error = ExampleError;
}

/// `follow_notes`: the notes that `add_note` keeps from now on.
#[derive(Debug)]
pub enum FollowNotesChannel {}

impl ChannelSpec for FollowNotesChannel {
    const NAME: &'static str = "follow_notes";
    const DOCS: &'static str = "The notes that the plugin keeps from now on, until it stops.";
    type Input = ();
    type Item = Note;
    type Error = ExampleError;
}

/// The plugin's command, its channel and the limit that the UI checks.
pub fn interface() -> Interface {
    Interface::new()
        .command::<AddNoteCommand>()
        .channel::<FollowNotesChannel>()
        .constant(
            "MAX_NOTE_CHARACTERS",
            "The longest note that the plugin keeps, in characters.",
            MAX_NOTE_CHARACTERS,
        )
}
