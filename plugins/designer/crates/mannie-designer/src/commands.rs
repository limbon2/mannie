//! The plugin's command and channel (plugins/designer/docs/README.md,
//! contracts), as specs of the plugin SDK (ADR-0034, decision 8), so the host
//! checks their input before the plugin's handlers run and answers
//! `plugin_off` while the plugin is off.

use mannie_plugin_sdk::contracts::UtcMillis;
use mannie_plugin_sdk::ipc::{
    ChannelSpec, CommandSpec, InputRefusal, Interface, PluginError, Validate,
};
use serde::{Deserialize, Serialize};
use specta::Type;

/// The longest name of a project, in characters.
pub const MAX_NAME_CHARACTERS: usize = 100;

/// A limit as a number for the UI's message.
fn limit(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Whether a short text that the owner wrote is not blank, has at most
/// `max_characters` characters and no control characters.
fn is_short_text(text: &str, max_characters: usize) -> bool {
    let trimmed = text.trim();
    let is_blank = trimmed.is_empty();
    let is_long = trimmed.chars().count() > max_characters;
    let has_control = text.chars().any(char::is_control);
    !is_blank && !is_long && !has_control
}

/// The argument of `create_project`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateProject {
    /// The project's name.
    pub name: String,
}

impl Validate for CreateProject {
    /// The name is not blank, has at most [`MAX_NAME_CHARACTERS`] characters
    /// and no control characters.
    fn validate(&self) -> Result<(), InputRefusal> {
        if !is_short_text(&self.name, MAX_NAME_CHARACTERS) {
            return Err(InputRefusal::new("name"));
        }

        Ok(())
    }
}

/// A project as the plugin's UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    /// The project.
    pub id: String,
    /// The name that the owner gave, which the UI shows as content.
    pub name: String,
    /// When the owner created it.
    pub created_at: UtcMillis,
}

/// The projects, the newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectList {
    /// Every project.
    pub projects: Vec<ProjectView>,
}

/// Why a command or channel of the plugin failed. The host's refusals, such
/// as `plugin_off`, come beside these (ADR-0034, decision 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DesignerError {
    /// Another project has the name under SQLite's ASCII case-insensitive comparison.
    #[error("another project has the name")]
    NameTaken,
    /// The store did not keep or read what the plugin asked.
    #[error("the projects could not be read or saved")]
    StorageFailed,
}

impl PluginError for DesignerError {
    /// The stable code of the error, for logs.
    fn code(&self) -> &'static str {
        match self {
            Self::NameTaken => "designer.name_taken",
            Self::StorageFailed => "designer.storage_failed",
        }
    }
}

/// `create_project`: a project with a name that no other project has.
#[derive(Debug)]
pub enum CreateProjectCommand {}

impl CommandSpec for CreateProjectCommand {
    const NAME: &'static str = "create_project";
    const DOCS: &'static str = "Creates a project with a name that no other project has.";
    type Input = CreateProject;
    type Output = ProjectView;
    type Error = DesignerError;
}

/// `follow_projects`: every project, then again after each change.
#[derive(Debug)]
pub enum FollowProjectsChannel {}

impl ChannelSpec for FollowProjectsChannel {
    const NAME: &'static str = "follow_projects";
    const DOCS: &'static str =
        "Every project, the newest first, then again after each change, until the plugin stops.";
    type Input = ();
    type Item = ProjectList;
    type Error = DesignerError;
}

/// The plugin's command, its channel and the limits that the UI checks.
pub fn interface() -> Interface {
    Interface::new()
        .command::<CreateProjectCommand>()
        .channel::<FollowProjectsChannel>()
        .constant(
            "MAX_NAME_CHARACTERS",
            "The longest name of a project, in characters.",
            limit(MAX_NAME_CHARACTERS),
        )
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::ipc::{InputRefusal, Validate};

    use super::{CreateProject, MAX_NAME_CHARACTERS};

    #[test]
    fn a_name_is_short_plain_text() {
        let named = |name: &str| CreateProject {
            name: name.to_owned(),
        };

        assert_eq!(named(" Landing page ").validate(), Ok(()));
        assert_eq!(named(&"é".repeat(MAX_NAME_CHARACTERS)).validate(), Ok(()));
        for refused in [
            String::new(),
            "  ".to_owned(),
            "a\nb".to_owned(),
            "x".repeat(MAX_NAME_CHARACTERS + 1),
        ] {
            assert_eq!(
                named(&refused).validate(),
                Err(InputRefusal::new("name")),
                "{refused:?}"
            );
        }
    }
}
