//! Approval requests and the owner's decisions
//! (docs/features/system-tools/operations-and-approvals.md, approvals).

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::{ConversationId, PartId, ProjectId};

use super::access::AccessMode;
use super::external::ToolServerEndpoint;
use super::program::{ProgramEffectKey, ProgramTarget};

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
    /// On a worker: a path lies outside the folders that the worker's ceiling
    /// allows (plugins/workers/docs/host-and-ceiling.md, the ceiling).
    OutsideAllowedFolders,
    /// On a worker: the ceiling's mode asks for this change, which lies
    /// inside the allowed folders.
    HostAsks,
    /// A command on a remote host, which no rule or grant allows (ADR-0036).
    RemoteCommand,
    /// A run of an installed program with an effect beyond the workspace,
    /// which no rule or grant allows (ADR-0038).
    Program,
    /// A call of a tool server's tool, which no grant allows (ADR-0043).
    ExternalTool,
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
        /// Whether the grant also covers the command in the background, as a
        /// grant from the approval of a command in the background does; one
        /// from the foreground covers only the foreground (ADR-0041, D40).
        #[serde(default)]
        background: bool,
    },
    /// The same command text on the same remote host, reached at the same
    /// endpoint (ADR-0036). A host whose address, port or user changed is
    /// another endpoint, which the grant does not cover.
    RemoteCommand {
        /// The plugin that reaches the host.
        plugin: String,
        /// The plugin's key for the host.
        target: String,
        /// The host's name when the owner allowed it, for display only.
        label: String,
        /// Where the command runs, such as `deploy@build-box.lan:22`.
        endpoint: String,
        /// The command's text.
        command: String,
        /// Whether the grant also covers the command in the background
        /// (ADR-0041, D40).
        #[serde(default)]
        background: bool,
    },
    /// Runs of the same installed program with the same arguments in the
    /// same working directory, whose effects are among the grant's
    /// (ADR-0038, decision 6).
    Program {
        /// The plugin whose tool runs the program.
        plugin: String,
        /// The program's name in the plugin's manifest.
        name: String,
        /// The program's canonical path.
        program: String,
        /// The arguments that the manifest fixes.
        fixed: Vec<String>,
        /// The plugin's arguments.
        arguments: Vec<String>,
        /// The canonical working directory.
        working_directory: String,
        /// The keys of the effects that the grant covers, sorted.
        effects: Vec<ProgramEffectKey>,
    },
    /// Calls of the same tool of the same tool server, at the same endpoint,
    /// with any arguments (ADR-0043, decision 4).
    ExternalTool {
        /// The plugin that reaches the server.
        plugin: String,
        /// The plugin's key for the server.
        target: String,
        /// The server's name when the owner allowed it, for display only.
        label: String,
        /// Where the server runs.
        endpoint: ToolServerEndpoint,
        /// The tool, as the server names it.
        tool: String,
    },
}

/// A remote host as a grant, a rule or a proposal names it: the plugin, its
/// key for the host and the endpoint, which decide what is covered, and the
/// host's name, which is for display only.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTarget {
    /// The plugin that reaches the host.
    pub plugin: String,
    /// The plugin's key for the host.
    pub target: String,
    /// The host's name when the owner saved the rule.
    pub label: String,
    /// Where commands run, such as `deploy@build-box.lan:22`.
    pub endpoint: String,
}

/// The rule that "Save rule" can make from a command: its words, of which the
/// owner keeps a leading part, and the folder the rule can be limited to; or,
/// for a command on a remote host, the host that the rule is always limited
/// to; or, for a run of an installed program, the program and the effects
/// that the rule covers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RuleProposal {
    /// The command's words, as the rule would match them. For a program's
    /// run, the program's name and the plugin's arguments.
    pub words: Vec<String>,
    /// The canonical working directory, to which the owner can limit the rule.
    /// Empty for a command on a remote host, which has none.
    pub folder: String,
    /// The remote host of the command, or `None` for a command on this
    /// machine. A rule from it applies only on that host (ADR-0036).
    #[serde(default)]
    pub remote: Option<RemoteTarget>,
    /// The installed program of the run, or `None` for a command. A rule
    /// from it applies only to that plugin's program, with the effects of
    /// the run (ADR-0038, decision 6).
    #[serde(default)]
    pub program: Option<ProgramTarget>,
    /// Whether the command runs in the background, so a rule from it also
    /// covers commands in the background (ADR-0041, D40).
    #[serde(default)]
    pub background: bool,
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
    /// The name of the worker whose host the operation runs on, or `None` on
    /// the master. A worker's request offers only "Allow once" and "Deny".
    #[serde(default)]
    pub host: Option<String>,
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
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
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
    /// On a worker: a read inside the allowed folders, or a change there in
    /// autonomous mode.
    AllowedFolder,
    /// On a worker in autonomous mode: a rule of the host allowed the command.
    HostRule,
    /// The conversation was in full access (ADR-0029): on the master its own
    /// switch, and on a worker the switch of the conversation on the master
    /// that delegated the task, which answered the request.
    FullAccess,
    /// On a worker whose ceiling is in full access mode (ADR-0029).
    HostFullAccess,
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
