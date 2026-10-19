//! A plugin's interface: the commands, channels and constants that its UI
//! uses, as the build grants them and the bindings exporter writes them.
//!
//! The interface holds each endpoint's name, documentation and input limit,
//! and the functions that describe its types to specta, so the exporter
//! reads the types without the plugin's code running.

use std::fmt;

use serde::Serialize;
use specta::datatype::DataType;

use super::{ChannelSpec, CommandSpec};

/// What specta knows of a type: the function that registers it in a
/// collection and returns its shape.
pub(crate) type Shape = fn(&mut specta::Types) -> DataType;

/// A command or a channel of an interface.
#[derive(Clone)]
#[non_exhaustive]
pub struct Endpoint {
    /// Its name.
    pub name: &'static str,
    /// What it does, for the generated function's documentation.
    pub docs: &'static str,
    /// The most bytes of its input as JSON.
    pub max_input_bytes: u32,
    /// The input's type.
    #[cfg_attr(
        not(feature = "bindings"),
        expect(dead_code, reason = "only the bindings exporter reads the types")
    )]
    pub(crate) input: Shape,
    /// The command's output, or the channel's item.
    #[cfg_attr(
        not(feature = "bindings"),
        expect(dead_code, reason = "only the bindings exporter reads the types")
    )]
    pub(crate) output: Shape,
    /// The plugin's error.
    #[cfg_attr(
        not(feature = "bindings"),
        expect(dead_code, reason = "only the bindings exporter reads the types")
    )]
    pub(crate) error: Shape,
    /// The Rust name of the error, for the exporter's messages.
    #[cfg_attr(
        not(feature = "bindings"),
        expect(dead_code, reason = "only the bindings exporter reads the types")
    )]
    pub(crate) error_name: &'static str,
}

impl fmt::Debug for Endpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Endpoint")
            .field("name", &self.name)
            .field("max_input_bytes", &self.max_input_bytes)
            .finish_non_exhaustive()
    }
}

/// A value that the UI also needs, such as a limit.
#[derive(Clone)]
#[non_exhaustive]
pub struct Constant {
    /// Its name in the generated file, such as `MAX_WORKER_NAME_CHARACTERS`.
    pub name: &'static str,
    /// What it is, for the generated constant's documentation.
    pub docs: &'static str,
    /// Its value as JSON.
    pub value: serde_json::Value,
    /// Its type.
    #[cfg_attr(
        not(feature = "bindings"),
        expect(dead_code, reason = "only the bindings exporter reads the types")
    )]
    pub(crate) shape: Shape,
}

impl fmt::Debug for Constant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Constant")
            .field("name", &self.name)
            .field("value", &self.value)
            .finish_non_exhaustive()
    }
}

/// A plugin's commands, channels and constants, in the order in which the
/// plugin added them.
#[derive(Debug, Clone, Default)]
pub struct Interface {
    commands: Vec<Endpoint>,
    channels: Vec<Endpoint>,
    constants: Vec<Constant>,
}

impl Interface {
    /// An interface without commands, channels or constants.
    pub fn new() -> Self {
        Self::default()
    }

    /// The interface with the command `C`.
    #[must_use]
    pub fn command<C: CommandSpec>(mut self) -> Self {
        self.commands.push(Endpoint {
            name: C::NAME,
            docs: C::DOCS,
            max_input_bytes: C::MAX_INPUT_BYTES,
            input: <C::Input as specta::Type>::definition,
            output: <C::Output as specta::Type>::definition,
            error: <C::Error as specta::Type>::definition,
            error_name: std::any::type_name::<C::Error>(),
        });
        self
    }

    /// The interface with the channel `C`.
    #[must_use]
    pub fn channel<C: ChannelSpec>(mut self) -> Self {
        self.channels.push(Endpoint {
            name: C::NAME,
            docs: C::DOCS,
            max_input_bytes: C::MAX_INPUT_BYTES,
            input: <C::Input as specta::Type>::definition,
            output: <C::Item as specta::Type>::definition,
            error: <C::Error as specta::Type>::definition,
            error_name: std::any::type_name::<C::Error>(),
        });
        self
    }

    /// The interface with the constant `name`, whose value the UI also
    /// needs, described by `docs`. A value that does not serialize is left
    /// out, and the log records it.
    #[must_use]
    pub fn constant<T: Serialize + specta::Type>(
        mut self,
        name: &'static str,
        docs: &'static str,
        value: T,
    ) -> Self {
        match serde_json::to_value(value) {
            Ok(value) => self.constants.push(Constant {
                name,
                docs,
                value,
                shape: T::definition,
            }),
            Err(error) => {
                tracing::error!(%error, constant = name, "a constant of a plugin's interface did not serialize and was left out");
            }
        }
        self
    }

    /// The commands.
    pub fn commands(&self) -> &[Endpoint] {
        &self.commands
    }

    /// The channels.
    pub fn channels(&self) -> &[Endpoint] {
        &self.channels
    }

    /// The constants.
    pub fn constants(&self) -> &[Constant] {
        &self.constants
    }

    /// The names of the commands, in order.
    pub fn command_names(&self) -> Vec<&'static str> {
        self.commands.iter().map(|endpoint| endpoint.name).collect()
    }

    /// The names of the channels, in order.
    pub fn channel_names(&self) -> Vec<&'static str> {
        self.channels.iter().map(|endpoint| endpoint.name).collect()
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::Interface;
    use crate::ipc::{ChannelSpec, CommandSpec, InputRefusal, PluginError, Validate};

    #[derive(Debug, Deserialize, specta::Type)]
    #[serde(deny_unknown_fields)]
    struct Note {
        text: String,
    }

    impl Validate for Note {
        fn validate(&self) -> Result<(), InputRefusal> {
            if self.text.is_empty() {
                return Err(InputRefusal::new("text"));
            }
            Ok(())
        }
    }

    #[derive(Debug, Serialize, specta::Type, thiserror::Error)]
    #[serde(tag = "code", rename_all = "snake_case")]
    enum NotesError {
        #[error("the note is too long")]
        TooLong,
    }

    impl PluginError for NotesError {
        fn code(&self) -> &'static str {
            "notes.too_long"
        }
    }

    enum AddNote {}

    impl CommandSpec for AddNote {
        const NAME: &'static str = "add_note";
        const DOCS: &'static str = "Adds a note.";
        const MAX_INPUT_BYTES: u32 = 1024;
        type Input = Note;
        type Output = ();
        type Error = NotesError;
    }

    enum FollowNotes {}

    impl ChannelSpec for FollowNotes {
        const NAME: &'static str = "follow_notes";
        const DOCS: &'static str = "Follows the notes.";
        type Input = ();
        type Item = Vec<String>;
        type Error = NotesError;
    }

    #[test]
    fn an_interface_lists_its_commands_channels_and_constants_in_order() {
        let interface = Interface::new()
            .command::<AddNote>()
            .channel::<FollowNotes>()
            .constant("MAX_NOTE_CHARACTERS", "The longest note.", 200_u32);

        assert_eq!(interface.command_names(), ["add_note"]);
        assert_eq!(interface.channel_names(), ["follow_notes"]);
        assert_eq!(interface.commands()[0].docs, "Adds a note.");
        assert_eq!(interface.commands()[0].max_input_bytes, 1024);
        assert_eq!(interface.channels()[0].max_input_bytes, 64 * 1024);
        assert_eq!(interface.constants()[0].name, "MAX_NOTE_CHARACTERS");
        assert_eq!(interface.constants()[0].docs, "The longest note.");
        assert_eq!(interface.constants()[0].value, serde_json::json!(200));
        assert_eq!(NotesError::TooLong.code(), "notes.too_long");
    }

    #[test]
    fn an_empty_interface_has_nothing() {
        let interface = Interface::new();

        assert!(interface.commands().is_empty());
        assert!(interface.channels().is_empty());
        assert!(interface.constants().is_empty());
    }
}
