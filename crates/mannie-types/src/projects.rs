//! The projects of the chat feature (docs/features/chat/projects.md): folders
//! that the owner added, whose conversations work in them (ADR-0023). This
//! module holds the view of a project, the arguments of the project commands
//! and the checks of a project's name. The errors are the chat feature's
//! [`ChatError`].

use std::path::Path;

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::chat::ChatError;
use crate::clock::UtcMillis;
use crate::ids::ProjectId;

/// The longest name the owner can give a project, in characters.
pub const MAX_PROJECT_NAME_CHARACTERS: usize = 100;

/// A project as the conversation list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProjectView {
    /// The project.
    pub id: ProjectId,
    /// Its name: the one the owner gave it, or the folder's name.
    pub name: String,
    /// The folder, with its links resolved.
    pub folder: String,
    /// When the owner added it.
    pub created_at: UtcMillis,
}

/// The argument of `add_project_from_path`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AddProjectFromPath {
    /// The folder as the owner typed it.
    pub folder: String,
}

/// The argument of `rename_project`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RenameProject {
    /// The project.
    pub project_id: ProjectId,
    /// The new name.
    pub name: String,
}

/// The argument of `remove_project`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoveProject {
    /// The project.
    pub project_id: ProjectId,
    /// How many conversations the owner confirmed to delete with it. When
    /// the project has another number of them, nothing is removed.
    pub conversations: u32,
}

/// Checks a name the owner chose and returns it without surrounding white
/// space. Line breaks and other control characters become spaces.
pub fn check_project_name(name: &str) -> Result<String, ChatError> {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    let length = trimmed.chars().count();
    if length == 0 || length > MAX_PROJECT_NAME_CHARACTERS {
        return Err(ChatError::ProjectNameInvalid {
            max_characters: u32::try_from(MAX_PROJECT_NAME_CHARACTERS).unwrap_or(u32::MAX),
        });
    }
    Ok(trimmed.to_owned())
}

/// The name of a project that the owner has not renamed: the last component
/// of its folder, or the whole path for a folder without one, such as a
/// drive's root.
pub fn folder_name(folder: &str) -> String {
    Path::new(folder).file_name().map_or_else(
        || folder.to_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::{MAX_PROJECT_NAME_CHARACTERS, check_project_name, folder_name};
    use crate::chat::ChatError;

    #[test]
    fn names_are_trimmed_and_lose_their_control_characters() {
        assert_eq!(
            check_project_name("  Mannie\tapp "),
            Ok("Mannie app".to_owned())
        );
        let invalid = Err(ChatError::ProjectNameInvalid {
            max_characters: 100,
        });
        assert_eq!(check_project_name(" \n "), invalid);
        assert_eq!(
            check_project_name(&"é".repeat(MAX_PROJECT_NAME_CHARACTERS + 1)),
            invalid
        );
        assert_eq!(
            check_project_name(&"é".repeat(MAX_PROJECT_NAME_CHARACTERS))
                .unwrap()
                .chars()
                .count(),
            MAX_PROJECT_NAME_CHARACTERS
        );
    }

    #[test]
    fn a_folder_names_its_project_by_its_last_component() {
        #[cfg(windows)]
        {
            assert_eq!(folder_name(r"D:\Programming\projects\mannie"), "mannie");
            assert_eq!(folder_name(r"D:\"), r"D:\");
        }
        #[cfg(not(windows))]
        {
            assert_eq!(folder_name("/home/owner/projects/mannie"), "mannie");
            assert_eq!(folder_name("/"), "/");
        }
    }
}
