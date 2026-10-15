//! The arguments, results and errors of the system tools feature's commands
//! (docs/features/system-tools/operations-and-approvals.md, contracts).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::clock::UtcMillis;
use crate::ids::{ArchiveId, ConversationId, GrantId, ProjectId, RuleId};

use super::approval::GrantScope;

/// A command rule as the settings list it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CommandRule {
    /// The rule.
    pub id: RuleId,
    /// The words a command must start with.
    pub words: Vec<String>,
    /// The folder the working directory must lie in, or `None` for anywhere.
    pub folder: Option<String>,
    /// When the owner saved it.
    pub created_at: UtcMillis,
}

/// The argument of `delete_command_rule`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RuleRef {
    /// The rule.
    pub rule_id: RuleId,
}

/// A grant of a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ConversationGrant {
    /// The grant.
    pub id: GrantId,
    /// What it allows.
    pub scope: GrantScope,
    /// When the owner allowed it.
    pub created_at: UtcMillis,
}

/// The argument of `revoke_conversation_grant`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct GrantRef {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The grant.
    pub grant_id: GrantId,
}

/// A grant for every conversation of a project, or for every conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SharedGrant {
    /// The grant.
    pub id: GrantId,
    /// The project whose conversations it covers, or `None` for every
    /// conversation.
    pub project_id: Option<ProjectId>,
    /// What it allows.
    pub scope: GrantScope,
    /// When the owner allowed it.
    pub created_at: UtcMillis,
}

/// The argument of `revoke_shared_grant`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SharedGrantRef {
    /// The grant.
    pub grant_id: GrantId,
}

/// The most lines one page of an archived output holds.
pub const MAX_OUTPUT_PAGE_LINES: u32 = 2_000;

/// The argument of `read_tool_output`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReadToolOutput {
    /// The conversation the output belongs to.
    pub conversation_id: ConversationId,
    /// The archived output.
    pub archive_id: ArchiveId,
    /// The first line to read, from 1.
    pub first_line: u32,
    /// How many lines to read, at most [`MAX_OUTPUT_PAGE_LINES`].
    pub limit: u32,
}

/// A page of an archived output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ToolOutputPage {
    /// The lines, without their line breaks.
    pub lines: Vec<String>,
    /// The number of the first line, from 1.
    pub first_line: u32,
    /// The lines of the whole output.
    pub total_lines: u32,
}

/// Why a command of the system tools feature failed. The UI shows the message
/// for the code from its catalog (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ToolsError {
    /// The conversation does not exist.
    #[error("the conversation does not exist")]
    ConversationNotFound,
    /// The tool call does not wait for an approval any more.
    #[error("the tool call does not wait for an approval")]
    ApprovalNotPending,
    /// The tool call waits for an approval of different accesses than the
    /// decision names.
    #[error("the approval request changed")]
    ApprovalChanged,
    /// A rule cannot cover this command, because it has shell metacharacters.
    #[error("a rule cannot cover this command")]
    RuleNotPossible,
    /// The rule would keep no word, or more words than the command has.
    #[error("the rule's prefix is not valid")]
    RulePrefixInvalid,
    /// "Allow for this project" in a conversation that belongs to no
    /// project.
    #[error("the conversation belongs to no project")]
    NotInAProject,
    /// "Allow for" a request whose operation can only be allowed once.
    #[error("the operation can only be allowed once")]
    GrantNotPossible,
    /// The rule does not exist.
    #[error("the rule does not exist")]
    RuleNotFound,
    /// The grant does not exist in this conversation.
    #[error("the grant does not exist")]
    GrantNotFound,
    /// The archived output does not exist in this conversation.
    #[error("the archived output does not exist")]
    OutputNotFound,
    /// An environment variable's name is not valid.
    #[error("{name:?} is not a valid name of an environment variable")]
    EnvironmentNameInvalid {
        /// The name, cut to 128 characters.
        name: String,
    },
    /// An environment variable's name looks like the name of a secret.
    #[error("{name:?} looks like the name of a secret")]
    EnvironmentNameSecret {
        /// The name.
        name: String,
    },
    /// More environment variables than Mannie accepts.
    #[error("more than {max} environment variables")]
    TooManyEnvironmentNames {
        /// The limit.
        max: u32,
    },
    /// The settings were written by a newer version of Mannie, which this
    /// version does not overwrite.
    #[error("the tools settings were written by a newer version of Mannie")]
    WrittenByNewerVersion,
    /// The store failed. The log has the cause.
    #[error("the data could not be read or saved")]
    StorageFailed,
    /// The core started without its database.
    #[error("the database is not open")]
    StoreUnavailable,
    /// Mannie is quitting.
    #[error("Mannie is quitting")]
    ShuttingDown,
}

impl ToolsError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::ConversationNotFound => "tools.conversation_not_found",
            Self::ApprovalNotPending => "tools.approval_not_pending",
            Self::ApprovalChanged => "tools.approval_changed",
            Self::RuleNotPossible => "tools.rule_not_possible",
            Self::RulePrefixInvalid => "tools.rule_prefix_invalid",
            Self::NotInAProject => "tools.not_in_a_project",
            Self::GrantNotPossible => "tools.grant_not_possible",
            Self::RuleNotFound => "tools.rule_not_found",
            Self::GrantNotFound => "tools.grant_not_found",
            Self::OutputNotFound => "tools.output_not_found",
            Self::EnvironmentNameInvalid { .. } => "tools.environment_name_invalid",
            Self::EnvironmentNameSecret { .. } => "tools.environment_name_secret",
            Self::TooManyEnvironmentNames { .. } => "tools.too_many_environment_names",
            Self::WrittenByNewerVersion => "tools.written_by_newer_version",
            Self::StorageFailed => "tools.storage_failed",
            Self::StoreUnavailable => "tools.store_unavailable",
            Self::ShuttingDown => "tools.shutting_down",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ToolsError;

    #[test]
    fn errors_serialize_with_their_code() {
        assert_eq!(
            serde_json::to_value(ToolsError::ApprovalChanged).unwrap(),
            serde_json::json!({ "code": "approval_changed" })
        );
        assert_eq!(
            serde_json::to_value(ToolsError::EnvironmentNameSecret {
                name: "GITHUB_TOKEN".to_owned()
            })
            .unwrap(),
            serde_json::json!({ "code": "environment_name_secret", "name": "GITHUB_TOKEN" })
        );
        assert_eq!(ToolsError::ApprovalChanged.code(), "tools.approval_changed");
    }
}
