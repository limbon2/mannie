//! The policy that authorizes operations (docs/features/system-tools/
//! operations-and-approvals.md, policy). It is a pure function of the
//! accesses and its inputs, which the runtime reads again for every call, so
//! a change of the project, the settings, a grant or a rule applies from the
//! next call.

use std::path::{Path, PathBuf};

use mannie_types::ceiling::CeilingMode;
use mannie_types::ids::ProjectId;
use mannie_types::tools::{
    Access, AccessMode, ApprovalReason, ApprovalRequest, CommandRule, Decision, GrantScope,
    Refusal, RemoteCommandRule, RemoteTarget, RuleProposal, ToolsError,
};

use crate::ceiling::Ceiling;
use crate::digest::digest;
use crate::paths::{is_inside, is_same_path};
use crate::projects::names_project;
use crate::rules::{command_words, remote_command_words, starts_with_words};

/// The grants that apply to a conversation, by how far they reach.
#[derive(Debug, Clone, Copy, Default)]
pub struct Grants<'a> {
    /// The conversation's own grants.
    pub conversation: &'a [GrantScope],
    /// The grants for every conversation of its project.
    pub project: &'a [GrantScope],
    /// The grants for every conversation.
    pub always: &'a [GrantScope],
}

/// What the policy decides on.
#[derive(Debug, Clone, Copy)]
pub struct PolicyInputs<'a> {
    /// The conversation's workspace, in canonical form.
    pub workspace: Option<&'a Path>,
    /// Whether changes inside the workspace run without asking.
    pub allow_changes_in_workspace: bool,
    /// Mannie's data and log folders, in canonical form. Every access inside
    /// them asks, so no grant or setting lets the model change the data that
    /// holds the policy (T-18).
    pub protected: &'a [PathBuf],
    /// The grants that apply to the conversation.
    pub grants: Grants<'a>,
    /// The command rules.
    pub rules: &'a [CommandRule],
    /// The rules for commands on remote hosts, each limited to its host
    /// (ADR-0036).
    pub remote_rules: &'a [RemoteCommandRule],
    /// The conversation's project, which "Allow for this project" would
    /// cover.
    pub project: Option<&'a ProjectId>,
    /// The owner's message of the turn, while no tool call of the turn came
    /// before this one, so nothing the model read since can stand in for the
    /// owner's words (ADR-0023, decision 3). A move into a project that it
    /// names runs without a question.
    pub owner_request: Option<&'a str>,
    /// On a worker, its policy ceiling, which takes the place of the
    /// workspace and the rules and can refuse an operation outright
    /// (ADR-0028). `None` on the master.
    pub ceiling: Option<&'a Ceiling>,
    /// On a worker, its name, which an approval request shows.
    pub host: Option<&'a str>,
    /// On the master, whether the conversation is in full access, which the
    /// owner switched on (ADR-0029): every operation runs without asking.
    /// On a worker, its ceiling's mode decides instead.
    pub full_access: bool,
}

/// What the policy decided about an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The call touches nothing that needs authorization.
    Unrestricted,
    /// The policy allows the operation. The decision names what allowed it:
    /// the workspace, a grant or a rule.
    Allowed(Decision),
    /// The owner decides. The request is boxed because it is far larger than
    /// the other verdicts.
    Ask(Box<ApprovalRequest>),
    /// A worker's ceiling forbids the operation; nobody can allow it.
    Refused(Refusal),
}

/// Decides about an operation. A conversation in full access allows every
/// operation, Mannie's own folders included (ADR-0029, decision 3).
/// Otherwise it is allowed only when every access is:
///
/// 1. an access inside Mannie's data or log folders asks;
/// 2. a read inside the workspace is allowed;
/// 3. a change inside the workspace is allowed when the settings say so;
/// 4. a grant of the conversation, of its project or for every conversation
///    allows what it covers;
/// 5. a rule allows a command it matches, and a rule of a remote host a
///    command on that host;
/// 6. a move into a project that the owner's request names is allowed;
/// 7. everything else asks.
///
/// Commands, here or on a remote host, are never allowed by the workspace, a
/// command with a metacharacter is never allowed by a grant or a rule, and a
/// move is never granted.
///
/// On a worker, its ceiling decides in place of the workspace and the rules
/// ([`decide_under_ceiling`]): an access that the ceiling forbids refuses the
/// whole operation, and a request offers only "Allow once" and "Deny".
pub fn decide(accesses: &[Access], inputs: &PolicyInputs<'_>) -> Verdict {
    if accesses.is_empty() {
        return Verdict::Unrestricted;
    }
    if inputs.full_access && inputs.ceiling.is_none() {
        return Verdict::Allowed(Decision::FullAccess);
    }
    let mut decision = if inputs.ceiling.is_some() {
        Decision::AllowedFolder
    } else {
        Decision::Workspace
    };
    let mut reasons: Vec<ApprovalReason> = Vec::new();
    let mut asking: Vec<&Access> = Vec::new();
    for access in accesses {
        let held = match inputs.ceiling {
            Some(ceiling) => decide_under_ceiling(access, ceiling, inputs),
            None => decide_access(access, inputs).map_err(Hold::Ask),
        };
        match held {
            Ok(source) => decision = stronger(decision, source),
            Err(Hold::Refuse(refusal)) => return Verdict::Refused(refusal),
            Err(Hold::Ask(reason)) => {
                if !reasons.contains(&reason) {
                    reasons.push(reason);
                }
                asking.push(access);
            }
        }
    }
    if asking.is_empty() {
        return Verdict::Allowed(decision);
    }
    if inputs.ceiling.is_some() {
        // A worker grants nothing for longer than once and saves no rule, so
        // nothing that the master sends widens its ceiling (ADR-0028).
        return Verdict::Ask(Box::new(ApprovalRequest {
            digest: digest(accesses),
            reasons,
            grants: Vec::new(),
            rule: None,
            project: None,
            host: inputs.host.map(str::to_owned),
        }));
    }
    let protected = reasons.contains(&ApprovalReason::MannieFolder);
    Verdict::Ask(Box::new(ApprovalRequest {
        digest: digest(accesses),
        grants: if protected {
            Vec::new()
        } else {
            grants_for(&asking)
        },
        rule: if protected { None } else { rule_for(accesses) },
        reasons,
        project: inputs.project.cloned(),
        host: None,
    }))
}

/// Why an access does not run without the owner.
enum Hold {
    /// The owner decides.
    Ask(ApprovalReason),
    /// Nobody can allow it.
    Refuse(Refusal),
}

/// What allows one access on a worker, why it asks, or why it is refused
/// (plugins/workers/docs/host-and-ceiling.md, the ceiling). Reads inside the
/// allowed folders run; read-only refuses every change and command; ask asks
/// for them; autonomous runs changes inside the allowed folders and the
/// commands that a host rule matches; everything else asks. The worker's own
/// folders always ask, and a grant of the task's conversation allows what it
/// covers, as on the master. Full access runs every path and command, the
/// worker's own folders included (ADR-0029, decision 4); a host has no
/// projects in any mode.
fn decide_under_ceiling(
    access: &Access,
    ceiling: &Ceiling,
    inputs: &PolicyInputs<'_>,
) -> Result<Decision, Hold> {
    let read_only = ceiling.mode == CeilingMode::ReadOnly;
    let autonomous = ceiling.mode == CeilingMode::Autonomous;
    let full_access = ceiling.mode == CeilingMode::FullAccess;
    let runs_anything = matches!(access, Access::Path { .. } | Access::Command { .. });
    if full_access && runs_anything {
        return Ok(Decision::HostFullAccess);
    }
    match access {
        Access::Path { path, mode, .. } => {
            let path = Path::new(path);
            if mode.changes() && read_only {
                return Err(Hold::Refuse(Refusal::ReadOnlyHost));
            }
            if is_protected(path, inputs) {
                return Err(Hold::Ask(ApprovalReason::MannieFolder));
            }
            let inside = ceiling
                .allowed_folders
                .iter()
                .any(|folder| is_inside(path, folder));
            if inside && (!mode.changes() || autonomous) {
                return Ok(Decision::AllowedFolder);
            }
            if let Some(decision) = granted(&inputs.grants, |grant| covers_path(grant, path, *mode))
            {
                return Ok(decision);
            }
            Err(Hold::Ask(if inside {
                ApprovalReason::HostAsks
            } else {
                ApprovalReason::OutsideAllowedFolders
            }))
        }
        Access::Command {
            command,
            working_directory,
            ..
        } => {
            if read_only {
                return Err(Hold::Refuse(Refusal::ReadOnlyHost));
            }
            let directory = Path::new(working_directory);
            if is_protected(directory, inputs) {
                return Err(Hold::Ask(ApprovalReason::MannieFolder));
            }
            let Some(words) = command_words(command) else {
                return Err(Hold::Ask(ApprovalReason::Command));
            };
            if let Some(decision) = granted(&inputs.grants, |grant| {
                covers_command(grant, command, directory)
            }) {
                return Ok(decision);
            }
            let ruled = autonomous
                && ceiling
                    .rules
                    .iter()
                    .any(|rule| rule_matches(rule, &words, directory));
            if ruled {
                return Ok(Decision::HostRule);
            }
            Err(Hold::Ask(ApprovalReason::Command))
        }
        Access::Project { .. } => Err(Hold::Refuse(Refusal::NoProjectsOnHost)),
        // A worker runs no plugins, so no tool of its own reaches a remote
        // host; the refusal keeps every access answered (ADR-0036).
        Access::RemoteCommand { .. } => Err(Hold::Refuse(Refusal::NoRemoteCommands)),
    }
}

/// What allows one access, or why it asks.
fn decide_access(access: &Access, inputs: &PolicyInputs<'_>) -> Result<Decision, ApprovalReason> {
    match access {
        Access::Path { path, mode, .. } => {
            let path = Path::new(path);
            if is_protected(path, inputs) {
                return Err(ApprovalReason::MannieFolder);
            }
            let in_workspace = inputs
                .workspace
                .is_some_and(|workspace| is_inside(path, workspace));
            if in_workspace && (!mode.changes() || inputs.allow_changes_in_workspace) {
                return Ok(Decision::Workspace);
            }
            if let Some(decision) = granted(&inputs.grants, |grant| covers_path(grant, path, *mode))
            {
                return Ok(decision);
            }
            Err(match (inputs.workspace, in_workspace) {
                (None, _) => ApprovalReason::NoWorkspace,
                (Some(_), true) => ApprovalReason::ChangeInWorkspace,
                (Some(_), false) => ApprovalReason::OutsideWorkspace,
            })
        }
        Access::Command {
            command,
            working_directory,
            ..
        } => {
            let directory = Path::new(working_directory);
            if is_protected(directory, inputs) {
                return Err(ApprovalReason::MannieFolder);
            }
            let Some(words) = command_words(command) else {
                return Err(ApprovalReason::Command);
            };
            if let Some(decision) = granted(&inputs.grants, |grant| {
                covers_command(grant, command, directory)
            }) {
                return Ok(decision);
            }
            if inputs
                .rules
                .iter()
                .any(|rule| rule_matches(rule, &words, directory))
            {
                return Ok(Decision::Rule);
            }
            Err(ApprovalReason::Command)
        }
        Access::Project { name, folder, .. } => {
            let named = inputs
                .owner_request
                .is_some_and(|request| names_project(request, name, folder));
            if named {
                Ok(Decision::OwnerNamedProject)
            } else {
                Err(ApprovalReason::MoveToProject)
            }
        }
        // The workspace and Mannie's own folders are this machine's, so
        // neither applies on a remote host (ADR-0036).
        Access::RemoteCommand {
            plugin,
            target,
            endpoint,
            command,
            ..
        } => {
            let Some(words) = remote_command_words(command) else {
                return Err(ApprovalReason::RemoteCommand);
            };
            let host = HostKey {
                plugin,
                target,
                endpoint,
            };
            if let Some(decision) = granted(&inputs.grants, |grant| {
                covers_remote_command(grant, &host, command)
            }) {
                return Ok(decision);
            }
            if inputs
                .remote_rules
                .iter()
                .any(|rule| host.is(&rule.target) && starts_with_words(&words, &rule.words))
            {
                return Ok(Decision::Rule);
            }
            Err(ApprovalReason::RemoteCommand)
        }
    }
}

/// The parts of a remote host that decide what a grant or a rule covers: its
/// plugin, the plugin's key for it and the endpoint, never its name.
struct HostKey<'a> {
    plugin: &'a str,
    target: &'a str,
    endpoint: &'a str,
}

impl HostKey<'_> {
    /// Whether a rule's or a proposal's target is this host.
    fn is(&self, target: &RemoteTarget) -> bool {
        target.plugin == self.plugin
            && target.target == self.target
            && target.endpoint == self.endpoint
    }
}

/// The grant that covers an access, from the narrowest to the widest.
fn granted(grants: &Grants<'_>, covers: impl Fn(&GrantScope) -> bool) -> Option<Decision> {
    let held = [
        (grants.conversation, Decision::Grant),
        (grants.project, Decision::ProjectGrant),
        (grants.always, Decision::AlwaysGrant),
    ];
    held.into_iter()
        .find(|(scopes, _decision)| scopes.iter().any(&covers))
        .map(|(_scopes, decision)| decision)
}

/// Whether a path lies inside Mannie's data or log folders.
fn is_protected(path: &Path, inputs: &PolicyInputs<'_>) -> bool {
    inputs
        .protected
        .iter()
        .any(|folder| is_inside(path, folder))
}

/// The decision that names the source that reaches furthest: full access,
/// then a rule, then a grant for every conversation, of the project and of
/// the conversation, then the owner's request and the workspace.
fn stronger(current: Decision, source: Decision) -> Decision {
    let rank = |decision: Decision| match decision {
        Decision::FullAccess | Decision::HostFullAccess => 6,
        Decision::Rule | Decision::HostRule => 5,
        Decision::AlwaysGrant => 4,
        Decision::ProjectGrant => 3,
        Decision::Grant => 2,
        Decision::OwnerNamedProject => 1,
        _ => 0,
    };
    if rank(source) > rank(current) {
        source
    } else {
        current
    }
}

/// Whether a grant covers a path access.
fn covers_path(grant: &GrantScope, path: &Path, mode: AccessMode) -> bool {
    match grant {
        GrantScope::Path {
            mode: granted,
            folder,
        } => *granted == mode && is_inside(path, Path::new(folder)),
        GrantScope::Command { .. } | GrantScope::RemoteCommand { .. } => false,
    }
}

/// Whether a grant covers a command in a working directory.
fn covers_command(grant: &GrantScope, command: &str, directory: &Path) -> bool {
    match grant {
        GrantScope::Command {
            command: granted,
            working_directory,
        } => granted == command && is_same_path(directory, Path::new(working_directory)),
        GrantScope::Path { .. } | GrantScope::RemoteCommand { .. } => false,
    }
}

/// Whether a grant covers a command on a remote host: the same command text
/// on the same host at the same endpoint.
fn covers_remote_command(grant: &GrantScope, host: &HostKey<'_>, command: &str) -> bool {
    match grant {
        GrantScope::RemoteCommand {
            plugin,
            target,
            endpoint,
            command: granted,
            ..
        } => {
            plugin == host.plugin
                && target == host.target
                && endpoint == host.endpoint
                && granted == command
        }
        GrantScope::Path { .. } | GrantScope::Command { .. } => false,
    }
}

/// Whether a rule allows a command.
fn rule_matches(rule: &CommandRule, words: &[String], directory: &Path) -> bool {
    starts_with_words(words, &rule.words)
        && rule
            .folder
            .as_ref()
            .is_none_or(|folder| is_inside(directory, Path::new(folder)))
}

/// What "Allow for" grants for the accesses that asked: nothing when one of
/// them cannot be granted.
fn grants_for(asking: &[&Access]) -> Vec<GrantScope> {
    let mut kept: Vec<GrantScope> = Vec::new();
    for access in asking {
        let Some(scope) = grant_scope(access) else {
            return Vec::new();
        };
        if kept.iter().any(|existing| scope_covers(existing, &scope)) {
            continue;
        }
        kept.retain(|existing| !scope_covers(&scope, existing));
        kept.push(scope);
    }
    kept
}

/// The grant that covers an access: its mode in the folder itself for a
/// folder, in the folder of a file; the same command in the same working
/// directory; or the same command on the same remote host.
fn grant_scope(access: &Access) -> Option<GrantScope> {
    match access {
        Access::Path {
            path, mode, folder, ..
        } => {
            let folder = if *folder {
                path.clone()
            } else {
                Path::new(path).parent()?.to_str()?.to_owned()
            };
            Some(GrantScope::Path {
                mode: *mode,
                folder,
            })
        }
        Access::Command {
            command,
            working_directory,
            ..
        } => {
            command_words(command)?;
            Some(GrantScope::Command {
                command: command.clone(),
                working_directory: working_directory.clone(),
            })
        }
        Access::Project { .. } => None,
        Access::RemoteCommand {
            plugin,
            target,
            label,
            endpoint,
            command,
            ..
        } => {
            remote_command_words(command)?;
            Some(GrantScope::RemoteCommand {
                plugin: plugin.clone(),
                target: target.clone(),
                label: label.clone(),
                endpoint: endpoint.clone(),
                command: command.clone(),
            })
        }
    }
}

/// Whether one grant covers everything another covers.
fn scope_covers(wider: &GrantScope, narrower: &GrantScope) -> bool {
    match (wider, narrower) {
        (
            GrantScope::Path { mode, folder },
            GrantScope::Path {
                mode: narrower_mode,
                folder: narrower_folder,
            },
        ) => mode == narrower_mode && is_inside(Path::new(narrower_folder), Path::new(folder)),
        (
            GrantScope::Command {
                command,
                working_directory,
            },
            GrantScope::Command {
                command: narrower_command,
                working_directory: narrower_directory,
            },
        ) => {
            command == narrower_command
                && is_same_path(Path::new(narrower_directory), Path::new(working_directory))
        }
        (
            GrantScope::RemoteCommand {
                plugin,
                target,
                endpoint,
                command,
                ..
            },
            GrantScope::RemoteCommand {
                plugin: narrower_plugin,
                target: narrower_target,
                endpoint: narrower_endpoint,
                command: narrower_command,
                ..
            },
        ) => {
            plugin == narrower_plugin
                && target == narrower_target
                && endpoint == narrower_endpoint
                && command == narrower_command
        }
        _ => false,
    }
}

/// The rule that "Save rule" can make: only for an operation that is one
/// command that a rule can cover, here or on a remote host.
fn rule_for(accesses: &[Access]) -> Option<RuleProposal> {
    match accesses {
        [
            Access::Command {
                command,
                working_directory,
                ..
            },
        ] => Some(RuleProposal {
            words: command_words(command)?,
            folder: working_directory.clone(),
            remote: None,
        }),
        [
            Access::RemoteCommand {
                plugin,
                target,
                label,
                endpoint,
                command,
                ..
            },
        ] => Some(RuleProposal {
            words: remote_command_words(command)?,
            folder: String::new(),
            remote: Some(RemoteTarget {
                plugin: plugin.clone(),
                target: target.clone(),
                label: label.clone(),
                endpoint: endpoint.clone(),
            }),
        }),
        _ => None,
    }
}

/// The rule that "Save rule" makes from a request's proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SavedRule {
    /// A rule for commands on this machine.
    Local {
        /// The leading words a command must start with.
        words: Vec<String>,
        /// The folder the working directory must lie in, or `None` for
        /// anywhere.
        folder: Option<String>,
    },
    /// A rule for commands on one remote host (ADR-0036).
    Remote {
        /// The host, which the rule is always limited to.
        target: RemoteTarget,
        /// The leading words a command must start with.
        words: Vec<String>,
    },
}

/// The rule that "Save rule" makes from a request's proposal, with the
/// leading words and the folder limit the owner chose. A rule for a remote
/// host is always limited to that host, so the folder limit does not apply.
/// The core builds the rule from the request it holds, so the UI cannot
/// widen it.
pub fn saved_rule(
    proposal: &RuleProposal,
    prefix_words: u32,
    limit_to_folder: bool,
) -> Result<SavedRule, ToolsError> {
    let count = usize::try_from(prefix_words).unwrap_or(usize::MAX);
    if count == 0 || count > proposal.words.len() {
        return Err(ToolsError::RulePrefixInvalid);
    }
    let words = proposal.words[..count].to_vec();
    Ok(match &proposal.remote {
        Some(target) => SavedRule::Remote {
            target: target.clone(),
            words,
        },
        None => SavedRule::Local {
            words,
            folder: limit_to_folder.then(|| proposal.folder.clone()),
        },
    })
}

#[cfg(test)]
mod tests;
