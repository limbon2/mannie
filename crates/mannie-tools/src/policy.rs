//! The policy that authorizes operations (docs/features/system-tools/
//! operations-and-approvals.md, policy). It is a pure function of the
//! accesses and its inputs, which the runtime reads again for every call, so
//! a change of the project, the settings, a grant or a rule applies from the
//! next call.

use std::path::{Path, PathBuf};

use mannie_types::ids::ProjectId;
use mannie_types::tools::{
    Access, AccessMode, ApprovalReason, ApprovalRequest, CommandRule, Decision, GrantScope,
    RuleProposal, ToolsError,
};

use crate::digest::digest;
use crate::paths::{is_inside, is_same_path};
use crate::projects::names_project;
use crate::rules::{command_words, starts_with_words};

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
    /// The conversation's project, which "Allow for this project" would
    /// cover.
    pub project: Option<&'a ProjectId>,
    /// The owner's message of the turn, while no tool call of the turn came
    /// before this one, so nothing the model read since can stand in for the
    /// owner's words (ADR-0023, decision 3). A move into a project that it
    /// names runs without a question.
    pub owner_request: Option<&'a str>,
}

/// What the policy decided about an operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The call touches nothing that needs authorization.
    Unrestricted,
    /// The policy allows the operation. The decision names what allowed it:
    /// the workspace, a grant or a rule.
    Allowed(Decision),
    /// The owner decides.
    Ask(ApprovalRequest),
}

/// Decides about an operation. It is allowed only when every access is:
///
/// 1. an access inside Mannie's data or log folders asks;
/// 2. a read inside the workspace is allowed;
/// 3. a change inside the workspace is allowed when the settings say so;
/// 4. a grant of the conversation, of its project or for every conversation
///    allows what it covers;
/// 5. a rule allows a command it matches;
/// 6. a move into a project that the owner's request names is allowed;
/// 7. everything else asks.
///
/// Commands are never allowed by the workspace, a command with a
/// metacharacter is never allowed by a grant or a rule, and a move is never
/// granted.
pub fn decide(accesses: &[Access], inputs: &PolicyInputs<'_>) -> Verdict {
    if accesses.is_empty() {
        return Verdict::Unrestricted;
    }
    let mut decision = Decision::Workspace;
    let mut reasons: Vec<ApprovalReason> = Vec::new();
    let mut asking: Vec<&Access> = Vec::new();
    for access in accesses {
        match decide_access(access, inputs) {
            Ok(source) => decision = stronger(decision, source),
            Err(reason) => {
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
    let protected = reasons.contains(&ApprovalReason::MannieFolder);
    Verdict::Ask(ApprovalRequest {
        digest: digest(accesses),
        grants: if protected {
            Vec::new()
        } else {
            grants_for(&asking)
        },
        rule: if protected { None } else { rule_for(accesses) },
        reasons,
        project: inputs.project.cloned(),
    })
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

/// The decision that names the source that reaches furthest: a rule, then a
/// grant for every conversation, of the project and of the conversation, then
/// the owner's request and the workspace.
fn stronger(current: Decision, source: Decision) -> Decision {
    let rank = |decision: Decision| match decision {
        Decision::Rule => 5,
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
        GrantScope::Command { .. } => false,
    }
}

/// Whether a grant covers a command in a working directory.
fn covers_command(grant: &GrantScope, command: &str, directory: &Path) -> bool {
    match grant {
        GrantScope::Command {
            command: granted,
            working_directory,
        } => granted == command && is_same_path(directory, Path::new(working_directory)),
        GrantScope::Path { .. } => false,
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
/// folder, in the folder of a file; or the same command in the same working
/// directory.
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
        _ => false,
    }
}

/// The rule that "Save rule" can make: only for an operation that is one
/// command that a rule can cover.
fn rule_for(accesses: &[Access]) -> Option<RuleProposal> {
    let [
        Access::Command {
            command,
            working_directory,
            ..
        },
    ] = accesses
    else {
        return None;
    };
    Some(RuleProposal {
        words: command_words(command)?,
        folder: working_directory.clone(),
    })
}

/// The words and the folder of the rule that "Save rule" makes from a
/// request's proposal, with the leading words and the folder limit the owner
/// chose. The core builds the rule from the request it holds, so the UI
/// cannot widen it.
pub fn saved_rule(
    proposal: &RuleProposal,
    prefix_words: u32,
    limit_to_folder: bool,
) -> Result<(Vec<String>, Option<String>), ToolsError> {
    let count = usize::try_from(prefix_words).unwrap_or(usize::MAX);
    if count == 0 || count > proposal.words.len() {
        return Err(ToolsError::RulePrefixInvalid);
    }
    let words = proposal.words[..count].to_vec();
    let folder = limit_to_folder.then(|| proposal.folder.clone());
    Ok((words, folder))
}

#[cfg(test)]
mod tests;
