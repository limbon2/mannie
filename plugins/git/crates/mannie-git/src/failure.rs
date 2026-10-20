//! Why a git tool did not do what a call asked (plugins/git/docs/tools.md,
//! every tool): a code, which the call's card words (ADR-0039), and a text
//! for the model that says what went wrong and what to do. A failure in a
//! preparation refuses the call with its code, and one in an execution ends
//! the call with it.

use mannie_plugin_sdk::points::tools::{ToolRefusal, ToolResult};

use crate::redact::redact;
use crate::version::Version;

/// The most characters of git's own message in a failure.
const MAX_MESSAGE_CHARS: usize = 2000;

/// Why a git tool did not do what a call asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GitFailure {
    /// git is not on Mannie's `PATH`.
    Missing,
    /// The installed git is older than [`Version::MINIMUM`].
    TooOld { found: Version },
    /// The folder is not inside a git repository.
    NotARepository { folder: String },
    /// The repository has no work tree.
    Bare { folder: String },
    /// git refuses the repository, because another user owns it.
    DubiousOwnership { folder: String },
    /// A revision names no commit or object.
    UnknownRevision { revision: String },
    /// An argument breaks a rule of the tool, which the text names.
    InvalidArgument { detail: String },
    /// git took longer than its time.
    TimedOut,
    /// A read that the place's policy would ask the owner about.
    ReadNotAllowed { folder: String },
    /// The repository changed after the owner allowed the call.
    RepositoryChanged,
    /// Another git process holds a lock file of the repository.
    Locked { path: String },
    /// git needs the owner's name and email for a commit.
    IdentityMissing,
    /// git refused an operation and changed nothing, such as a merge that
    /// would overwrite local changes.
    Refused { operation: String, message: String },
    /// git failed with this message.
    Failed { message: String },
}

impl GitFailure {
    /// The failure of git's message, cleaned of credentials and cut.
    pub(crate) fn failed(message: &str) -> Self {
        Self::Failed {
            message: cleaned(message),
        }
    }

    /// The refusal of `operation` with git's message, cleaned and cut.
    pub(crate) fn refused(operation: &str, message: &str) -> Self {
        Self::Refused {
            operation: operation.to_owned(),
            message: cleaned(message),
        }
    }

    /// The code that the call's card words.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Missing => "git_missing",
            Self::TooOld { .. } => "git_too_old",
            Self::NotARepository { .. } => "not_a_repository",
            Self::Bare { .. } => "bare_repository",
            Self::DubiousOwnership { .. } => "dubious_ownership",
            Self::UnknownRevision { .. } => "unknown_revision",
            Self::InvalidArgument { .. } => "invalid_argument",
            Self::TimedOut => "timed_out",
            Self::ReadNotAllowed { .. } => "read_not_allowed",
            Self::RepositoryChanged => "repository_changed",
            Self::Locked { .. } => "locked",
            Self::IdentityMissing => "identity_missing",
            Self::Refused { .. } => "operation_refused",
            Self::Failed { .. } => "git_failed",
        }
    }

    /// What the model reads.
    pub(crate) fn text(&self) -> String {
        match self {
            Self::Missing => "git was not found on Mannie's PATH. Tell the owner that the git tools need git 2.40 or later.".to_owned(),
            Self::TooOld { found } => format!(
                "The installed git is {found}, and the git tools need 2.40 or later. Tell the owner."
            ),
            Self::NotARepository { folder } => {
                format!("{folder} is not inside a git repository.")
            }
            Self::Bare { folder } => format!(
                "{folder} is a bare repository, which has no work tree. The git tools work in repositories with one."
            ),
            Self::DubiousOwnership { folder } => format!(
                "git refuses the repository at {folder}, because another user owns its folder. Tell the owner; the git tools never change git's configuration."
            ),
            Self::UnknownRevision { revision } => {
                format!("{revision} names no commit or object in this repository.")
            }
            Self::InvalidArgument { detail } => detail.clone(),
            Self::TimedOut => "git did not finish in time. Try a narrower call, such as fewer commits or paths.".to_owned(),
            Self::ReadNotAllowed { folder } => format!(
                "Reading {folder} needs the owner's permission, which this view cannot ask for."
            ),
            Self::RepositoryChanged => "The repository changed after this call was allowed: HEAD, a branch or the index moved. Nothing was done. Read the repository again and call the tool again.".to_owned(),
            Self::Locked { path } => format!(
                "Another git process holds {path}, so nothing was done. Try again when it ends. If no git process runs, tell the owner, who can delete the lock file; the git tools never delete one."
            ),
            Self::IdentityMissing => "git needs the owner's name and email for commits, and its configuration has neither. Tell the owner; the git tools never change git's configuration.".to_owned(),
            Self::Refused { operation, message } => format!(
                "git refused the {operation} and changed nothing: {message}"
            ),
            Self::Failed { message } => format!("git failed: {message}"),
        }
    }

    /// The refusal of a call whose preparation failed.
    pub(crate) fn refusal(&self) -> ToolRefusal {
        ToolRefusal::new(self.text()).with_code(self.code())
    }

    /// The result of a call whose execution failed.
    pub(crate) fn result(&self) -> ToolResult {
        ToolResult::new(self.text()).with_failure(self.code())
    }
}

/// git's message without credentials, trimmed and cut to
/// [`MAX_MESSAGE_CHARS`].
fn cleaned(message: &str) -> String {
    redact(message.trim())
        .chars()
        .take(MAX_MESSAGE_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::GitFailure;
    use crate::version::Version;

    #[test]
    fn every_failure_has_its_own_code_of_lower_case_words() {
        let failures = [
            GitFailure::Missing,
            GitFailure::TooOld {
                found: Version::new(2, 30, 0),
            },
            GitFailure::NotARepository {
                folder: "D:/work".to_owned(),
            },
            GitFailure::Bare {
                folder: "D:/work".to_owned(),
            },
            GitFailure::DubiousOwnership {
                folder: "D:/work".to_owned(),
            },
            GitFailure::UnknownRevision {
                revision: "nope".to_owned(),
            },
            GitFailure::InvalidArgument {
                detail: "bad".to_owned(),
            },
            GitFailure::TimedOut,
            GitFailure::ReadNotAllowed {
                folder: "D:/work".to_owned(),
            },
            GitFailure::RepositoryChanged,
            GitFailure::Locked {
                path: "D:/work/.git/index.lock".to_owned(),
            },
            GitFailure::IdentityMissing,
            GitFailure::refused("merge", "local changes would be overwritten"),
            GitFailure::failed("boom"),
        ];

        let mut codes: Vec<&str> = failures.iter().map(GitFailure::code).collect();
        for code in &codes {
            assert!(
                code.chars()
                    .all(|character| character.is_ascii_lowercase() || character == '_'),
                "{code}"
            );
        }
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), failures.len());
        assert!(failures.iter().all(|failure| !failure.text().is_empty()));
    }

    #[test]
    fn git_s_message_loses_its_credentials_and_is_cut() {
        let failure =
            GitFailure::failed("fatal: unable to access 'https://me:token@example.com/r/'\n");
        let long = GitFailure::failed(&"x".repeat(5000));

        assert_eq!(
            failure,
            GitFailure::Failed {
                message: "fatal: unable to access 'https://example.com/r/'".to_owned()
            }
        );
        assert!(long.text().len() < 2100);
        assert_eq!(failure.result().failure.as_deref(), Some("git_failed"));
        assert_eq!(failure.refusal().code.as_deref(), Some("git_failed"));
    }
}
