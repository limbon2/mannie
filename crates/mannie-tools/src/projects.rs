//! What the tools know about the owner's projects (docs/features/chat/
//! projects.md, moving a conversation into a project; ADR-0023): the
//! conversation's project and the owner's projects, read again for every
//! call, the capability that moves a general conversation into a project,
//! and whether the owner's request names a project.
//!
//! The chat feature owns projects. The runtime provides the move, because a
//! move changes the conversation and is published like every change of it.

use std::fmt;
use std::sync::Arc;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_types::ids::{ConversationId, ProjectId};
use mannie_types::projects::{ProjectView, folder_name};

/// Why a move did not happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoveFailure {
    /// The project does not exist, or it is being removed.
    ProjectNotFound,
    /// The conversation already belongs to a project.
    AlreadyInProject {
        /// The project's name.
        name: String,
    },
    /// The store failed. The log has the cause.
    Storage,
}

/// Moves a general conversation into a project.
pub trait ProjectMoves: Send + Sync {
    /// Moves the conversation and returns its project.
    fn move_into(
        &self,
        conversation: ConversationId,
        project: ProjectId,
    ) -> BoxFuture<'static, Result<ProjectView, MoveFailure>>;
}

/// The moves of surroundings without projects: every move fails.
struct NoMoves;

impl ProjectMoves for NoMoves {
    fn move_into(
        &self,
        _conversation: ConversationId,
        _project: ProjectId,
    ) -> BoxFuture<'static, Result<ProjectView, MoveFailure>> {
        async { Err(MoveFailure::ProjectNotFound) }.boxed()
    }
}

/// The projects as one tool call sees them.
#[derive(Clone)]
pub struct ProjectFacts {
    /// The conversation's project, or `None` for a general conversation.
    pub current: Option<ProjectView>,
    /// Every project of the owner.
    pub all: Arc<[ProjectView]>,
    /// Moves the conversation into a project.
    pub moves: Arc<dyn ProjectMoves>,
}

impl ProjectFacts {
    /// Surroundings without projects, in which a move fails.
    pub fn none() -> Self {
        Self {
            current: None,
            all: Arc::from(Vec::new()),
            moves: Arc::new(NoMoves),
        }
    }
}

impl fmt::Debug for ProjectFacts {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectFacts")
            .field("current", &self.current)
            .field("all", &self.all.len())
            .finish_non_exhaustive()
    }
}

/// What decides whether a tool is offered to one model call.
#[derive(Debug, Clone, Copy)]
pub struct Availability<'a> {
    /// The conversation's project, or `None` for a general conversation.
    pub project: Option<&'a ProjectView>,
    /// Every project of the owner.
    pub projects: &'a [ProjectView],
}

/// Whether the owner's request names a project: the project's name or its
/// folder's own name as a whole word, or the folder's path, without regard to
/// case. Paths compare with either separator, and on Windows without regard
/// to case.
pub(crate) fn names_project(request: &str, name: &str, folder: &str) -> bool {
    let lowered = request.to_lowercase();
    let own_name = folder_name(folder);
    let named = [name, own_name.as_str()]
        .into_iter()
        .any(|word| contains_word(&lowered, &word.trim().to_lowercase()));
    named || contains_path(request, folder)
}

/// Letters, digits and underscores, which continue a word.
fn is_word_character(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

/// Whether `text` contains `word` with no word character right before or
/// after it.
fn contains_word(text: &str, word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    text.match_indices(word).any(|(start, matched)| {
        let before = text[..start].chars().next_back();
        let after = text[start + matched.len()..].chars().next();
        let starts_a_word = !before.is_some_and(is_word_character);
        let ends_a_word = !after.is_some_and(is_word_character);
        starts_a_word && ends_a_word
    })
}

/// A path in the form that [`contains_path`] compares.
fn comparable_path(path: &str) -> String {
    let unified = path.replace('\\', "/");
    if cfg!(windows) {
        unified.to_lowercase()
    } else {
        unified
    }
}

/// Whether `text` contains the folder's path, or a path inside it.
fn contains_path(text: &str, folder: &str) -> bool {
    let compared = comparable_path(folder);
    let folder = compared.trim_end_matches('/');
    if folder.is_empty() {
        return false;
    }
    contains_word(&comparable_path(text), folder)
}

#[cfg(test)]
mod tests {
    use super::names_project;

    #[cfg(windows)]
    const FOLDER: &str = r"D:\Programming\projects\mannie";
    #[cfg(not(windows))]
    const FOLDER: &str = "/home/owner/projects/mannie";

    #[test]
    fn a_request_names_a_project_by_its_name_as_a_word_in_any_case() {
        let folder = FOLDER.replace("mannie", "shell");
        assert!(names_project(
            "Fix the build in Mannie app.",
            "Mannie app",
            &folder
        ));
        assert!(names_project(
            "look at MANNIE APP, please",
            "Mannie app",
            &folder
        ));
        assert!(names_project("(mannie app)", "Mannie app", &folder));
        assert!(!names_project(
            "the mannie apps are fine",
            "Mannie app",
            &folder
        ));
        assert!(!names_project(
            "work on something else",
            "Mannie app",
            &folder
        ));
    }

    #[test]
    fn a_request_names_a_project_by_its_folders_own_name() {
        assert!(names_project(
            "Open mannie and run the tests",
            "Desktop",
            FOLDER
        ));
        assert!(!names_project(
            "Open mannies and run the tests",
            "Desktop",
            FOLDER
        ));
        assert!(!names_project("Open my_mannie folder", "Desktop", FOLDER));
    }

    #[test]
    fn a_request_names_a_project_by_its_folder_or_a_path_inside_it() {
        #[cfg(windows)]
        {
            assert!(names_project(
                r"read d:\programming\projects\mannie\Cargo.toml",
                "Desktop",
                FOLDER
            ));
            assert!(names_project(
                "read D:/Programming/projects/mannie/",
                "Desktop",
                FOLDER
            ));
            assert!(!names_project(
                r"read D:\Programming\projects\manniex",
                "Desktop",
                FOLDER
            ));
        }
        #[cfg(not(windows))]
        {
            assert!(names_project(
                "read /home/owner/projects/mannie/Cargo.toml",
                "Desktop",
                FOLDER
            ));
            assert!(!names_project(
                "read /home/owner/projects/Mannie2",
                "Desktop",
                FOLDER
            ));
        }
    }

    #[test]
    fn an_empty_name_names_nothing() {
        assert!(!names_project("anything at all", "  ", "/"));
    }
}
