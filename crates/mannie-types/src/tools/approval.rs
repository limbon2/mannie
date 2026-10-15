//! Approval requests and the owner's decisions
//! (docs/features/system-tools/operations-and-approvals.md, approvals).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::{ConversationId, PartId, ProjectId};

use super::access::AccessMode;

/// Why the policy asks the owner about an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalReason {
    /// A path lies outside the conversation's workspace.
    OutsideWorkspace,
    /// The operation changes something inside the workspace, and the settings
    /// do not allow that without asking.
    ChangeInWorkspace,
    /// A path lies inside Mannie's own data or log folders.
    MannieFolder,
    /// A command, which no rule or grant allows.
    Command,
    /// The conversation has no workspace.
    NoWorkspace,
    /// The model moves the conversation into a project, and the owner's
    /// message of the turn did not name it, or another tool call came first
    /// (docs/features/chat/projects.md, moving a conversation into a project).
    MoveToProject,
}

/// What a grant covers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum GrantScope {
    /// One access mode in a folder and everything below it.
    Path {
        /// The mode.
        mode: AccessMode,
        /// The canonical folder.
        folder: String,
    },
    /// The same command text in the same working directory.
    Command {
        /// The command's text.
        command: String,
        /// The canonical working directory.
        working_directory: String,
    },
}

/// The rule that "Save rule" can make from a command: its words, of which the
/// owner keeps a leading part, and the folder the rule can be limited to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RuleProposal {
    /// The command's words, as the rule would match them.
    pub words: Vec<String>,
    /// The canonical working directory, to which the owner can limit the rule.
    pub folder: String,
}

/// A question to the owner about one operation. It shows exactly what will
/// happen: the accesses are in the tool call's document, and the request adds
/// why the policy asks and what the options would allow.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    /// The digest of the accesses the request shows. A decision names it, so
    /// it answers only this request (T-5).
    pub digest: String,
    /// Why the policy asks.
    pub reasons: Vec<ApprovalReason>,
    /// What "Allow for" this conversation, its project or every conversation
    /// would allow, or nothing when the operation can only be allowed once.
    pub grants: Vec<GrantScope>,
    /// The rule that "Save rule" can make, for commands that a rule can cover.
    pub rule: Option<RuleProposal>,
    /// The project whose conversations "Allow for this project" would
    /// cover, or `None` in a general conversation.
    #[serde(default)]
    pub project: Option<ProjectId>,
}

/// The owner's answer to an approval request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(
    tag = "choice",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ApprovalChoice {
    /// Run this operation, and ask again next time.
    AllowOnce,
    /// Run it, and allow what the request's grants cover for the rest of the
    /// conversation.
    AllowForConversation,
    /// Run it, and allow what the request's grants cover in every
    /// conversation of the conversation's project.
    AllowForProject,
    /// Run it, and allow what the request's grants cover in every
    /// conversation.
    AllowAlways,
    /// Run it, and save a rule from its command.
    SaveRule {
        /// How many of the command's leading words the rule keeps, from 1.
        prefix_words: u32,
        /// Whether the rule applies only in the command's working directory
        /// and below it.
        limit_to_folder: bool,
    },
    /// Do not run it. The model learns that the owner denied it.
    Deny,
}

/// The argument of `decide_approval`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DecideApproval {
    /// The conversation.
    pub conversation_id: ConversationId,
    /// The tool call's part.
    pub part_id: PartId,
    /// The digest of the request the owner answers.
    pub digest: String,
    /// The answer.
    pub choice: ApprovalChoice,
}

/// What allowed or denied an operation, and when.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// The workspace allowed it.
    Workspace,
    /// A grant of the conversation allowed it.
    Grant,
    /// A grant of the conversation's project allowed it.
    ProjectGrant,
    /// A grant for every conversation allowed it.
    AlwaysGrant,
    /// A rule allowed it.
    Rule,
    /// The owner allowed it once.
    AllowedOnce,
    /// The owner allowed it for the conversation.
    AllowedForConversation,
    /// The owner allowed it for the conversation's project.
    AllowedForProject,
    /// The owner allowed it for every conversation.
    AllowedAlways,
    /// The owner's message of the turn named the project that the
    /// conversation moved into.
    OwnerNamedProject,
    /// The owner saved a rule and allowed it.
    RuleSaved,
    /// The owner denied it.
    Denied,
}

impl Decision {
    /// The decision that the owner's choice makes.
    pub fn of_choice(choice: &ApprovalChoice) -> Self {
        match choice {
            ApprovalChoice::AllowOnce => Self::AllowedOnce,
            ApprovalChoice::AllowForConversation => Self::AllowedForConversation,
            ApprovalChoice::AllowForProject => Self::AllowedForProject,
            ApprovalChoice::AllowAlways => Self::AllowedAlways,
            ApprovalChoice::SaveRule { .. } => Self::RuleSaved,
            ApprovalChoice::Deny => Self::Denied,
        }
    }

    /// Whether the operation may run.
    pub fn allows(self) -> bool {
        !matches!(self, Self::Denied)
    }
}

#[cfg(test)]
mod tests {
    use super::{ApprovalChoice, Decision};

    #[test]
    fn choices_serialize_with_their_name() {
        let save = ApprovalChoice::SaveRule {
            prefix_words: 2,
            limit_to_folder: true,
        };

        assert_eq!(
            serde_json::to_value(&save).unwrap(),
            serde_json::json!({ "choice": "save_rule", "prefixWords": 2, "limitToFolder": true })
        );
        let back: ApprovalChoice =
            serde_json::from_value(serde_json::json!({ "choice": "deny" })).unwrap();
        assert_eq!(back, ApprovalChoice::Deny);
        assert_eq!(
            serde_json::to_value(ApprovalChoice::AllowForProject).unwrap(),
            serde_json::json!({ "choice": "allow_for_project" })
        );
    }

    #[test]
    fn only_a_denial_stops_the_operation() {
        let choices = [
            ApprovalChoice::AllowOnce,
            ApprovalChoice::AllowForConversation,
            ApprovalChoice::AllowForProject,
            ApprovalChoice::AllowAlways,
            ApprovalChoice::SaveRule {
                prefix_words: 1,
                limit_to_folder: false,
            },
        ];
        for choice in &choices {
            assert!(Decision::of_choice(choice).allows(), "{choice:?}");
        }
        assert!(!Decision::of_choice(&ApprovalChoice::Deny).allows());
    }
}
