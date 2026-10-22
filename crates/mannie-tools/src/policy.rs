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
    ProgramEffectKey, ProgramRule, ProgramTarget, Refusal, RemoteCommandRule, RemoteTarget,
    RuleProposal, ToolsError, effect_keys,
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
    /// The rules for runs of installed programs, each limited to its
    /// plugin's program and to the effects of the run that it was saved from
    /// (ADR-0038).
    pub program_rules: &'a [ProgramRule],
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
/// 5. a rule allows a command it matches, a rule of a remote host a command
///    on that host, and a rule of an installed program a run of that
///    program with the effects that the rule covers;
/// 6. a move into a project that the owner's request names is allowed;
/// 7. everything else asks.
///
/// Commands, here or on a remote host, and runs of installed programs are
/// never allowed by the workspace, a command with a metacharacter is never
/// allowed by a grant or a rule, and a move is never granted.
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
            background,
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
                covers_command(grant, command, directory, *background)
            }) {
                return Ok(decision);
            }
            // The host's rules come from its configuration, and a ceiling
            // that allows a command allows it in the background too, for as
            // long as its task runs (ADR-0041, decision 9).
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
        // host or runs a plugin's program; the refusals keep every access
        // answered (ADR-0036, ADR-0038).
        Access::RemoteCommand { .. } => Err(Hold::Refuse(Refusal::NoRemoteCommands)),
        Access::Program { .. } => Err(Hold::Refuse(Refusal::NoProgramsOnHost)),
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
            background,
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
                covers_command(grant, command, directory, *background)
            }) {
                return Ok(decision);
            }
            let ruled = inputs.rules.iter().any(|rule| {
                covers_mode(rule.background, *background) && rule_matches(rule, &words, directory)
            });
            if ruled {
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
            background,
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
                covers_remote_command(grant, &host, command, *background)
            }) {
                return Ok(decision);
            }
            let ruled = inputs.remote_rules.iter().any(|rule| {
                covers_mode(rule.background, *background)
                    && host.is(&rule.target)
                    && starts_with_words(&words, &rule.words)
            });
            if ruled {
                return Ok(Decision::Rule);
            }
            Err(ApprovalReason::RemoteCommand)
        }
        Access::Program { .. } => decide_program(access, inputs),
    }
}

/// What allows a run of an installed program, or why it asks: a grant or a
/// rule that covers it, never the workspace, and nothing in Mannie's own
/// folders, where it asks and offers no grant (T-18).
fn decide_program(access: &Access, inputs: &PolicyInputs<'_>) -> Result<Decision, ApprovalReason> {
    let Some(run) = RunKey::of(access) else {
        return Err(ApprovalReason::Program);
    };
    if is_protected(Path::new(run.working_directory), inputs) {
        return Err(ApprovalReason::MannieFolder);
    }
    if let Some(decision) = granted(&inputs.grants, |grant| covers_program(grant, &run)) {
        return Ok(decision);
    }
    if inputs
        .program_rules
        .iter()
        .any(|rule| program_rule_matches(rule, &run))
    {
        return Ok(Decision::Rule);
    }
    Err(ApprovalReason::Program)
}

/// The parts of a run of an installed program that decide what a grant or a
/// rule covers: its plugin, the program's name and path, the arguments, the
/// working directory and the keys of the effects. The effects' counts and
/// items and the run's state never count (ADR-0038, decision 6).
struct RunKey<'a> {
    plugin: &'a str,
    name: &'a str,
    program: &'a str,
    fixed: &'a [String],
    arguments: &'a [String],
    working_directory: &'a str,
    effects: Vec<ProgramEffectKey>,
}

impl<'a> RunKey<'a> {
    /// The key of a program's run, or `None` for another access.
    fn of(access: &'a Access) -> Option<Self> {
        let Access::Program {
            plugin,
            name,
            program,
            fixed,
            arguments,
            working_directory,
            effects,
            ..
        } = access
        else {
            return None;
        };
        Some(Self {
            plugin,
            name,
            program,
            fixed,
            arguments,
            working_directory,
            effects: effect_keys(effects),
        })
    }

    /// The words that a rule matches: the program's name, then the plugin's
    /// arguments.
    fn words(&self) -> Vec<String> {
        let mut words = vec![self.name.to_owned()];
        words.extend(self.arguments.iter().cloned());
        words
    }
}

/// Whether every effect of `effects` is among `covered`.
fn covers_effects(covered: &[ProgramEffectKey], effects: &[ProgramEffectKey]) -> bool {
    effects.iter().all(|effect| covered.contains(effect))
}

/// Whether a rule allows a program's run: the same plugin and program,
/// words that the run's name and arguments start with, a working directory
/// inside the rule's folder, and only effects that the rule covers.
fn program_rule_matches(rule: &ProgramRule, run: &RunKey<'_>) -> bool {
    let target = &rule.target;
    let same_program =
        target.plugin == run.plugin && target.name == run.name && target.program == run.program;
    let in_folder = rule
        .folder
        .as_ref()
        .is_none_or(|folder| is_inside(Path::new(run.working_directory), Path::new(folder)));
    same_program
        && in_folder
        && starts_with_words(&run.words(), &rule.words)
        && covers_effects(&target.effects, &run.effects)
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
        GrantScope::Command { .. }
        | GrantScope::RemoteCommand { .. }
        | GrantScope::Program { .. } => false,
    }
}

/// Whether a grant or a rule of one mode covers a command of another: one
/// given or saved from a command in the background covers both modes, and
/// one from the foreground only the foreground (ADR-0041, D40).
fn covers_mode(granted_background: bool, background: bool) -> bool {
    granted_background || !background
}

/// Whether a grant covers a command in a working directory, in its mode.
fn covers_command(grant: &GrantScope, command: &str, directory: &Path, background: bool) -> bool {
    match grant {
        GrantScope::Command {
            command: granted,
            working_directory,
            background: granted_background,
        } => {
            granted == command
                && is_same_path(directory, Path::new(working_directory))
                && covers_mode(*granted_background, background)
        }
        GrantScope::Path { .. } | GrantScope::RemoteCommand { .. } | GrantScope::Program { .. } => {
            false
        }
    }
}

/// Whether a grant covers a command on a remote host: the same command text
/// on the same host at the same endpoint, in its mode.
fn covers_remote_command(
    grant: &GrantScope,
    host: &HostKey<'_>,
    command: &str,
    background: bool,
) -> bool {
    match grant {
        GrantScope::RemoteCommand {
            plugin,
            target,
            endpoint,
            command: granted,
            background: granted_background,
            ..
        } => {
            plugin == host.plugin
                && target == host.target
                && endpoint == host.endpoint
                && granted == command
                && covers_mode(*granted_background, background)
        }
        GrantScope::Path { .. } | GrantScope::Command { .. } | GrantScope::Program { .. } => false,
    }
}

/// Whether a grant covers a program's run: the same plugin, program,
/// arguments and working directory, and only effects that the grant covers.
fn covers_program(grant: &GrantScope, run: &RunKey<'_>) -> bool {
    match grant {
        GrantScope::Program {
            plugin,
            name,
            program,
            fixed,
            arguments,
            working_directory,
            effects,
        } => {
            plugin == run.plugin
                && name == run.name
                && program == run.program
                && fixed.as_slice() == run.fixed
                && arguments.as_slice() == run.arguments
                && is_same_path(
                    Path::new(run.working_directory),
                    Path::new(working_directory),
                )
                && covers_effects(effects, &run.effects)
        }
        GrantScope::Path { .. } | GrantScope::Command { .. } | GrantScope::RemoteCommand { .. } => {
            false
        }
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
/// directory; the same command on the same remote host; or the same run of
/// an installed program with its effects.
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
            background,
            ..
        } => {
            command_words(command)?;
            Some(GrantScope::Command {
                command: command.clone(),
                working_directory: working_directory.clone(),
                background: *background,
            })
        }
        Access::Project { .. } => None,
        Access::RemoteCommand {
            plugin,
            target,
            label,
            endpoint,
            command,
            background,
            ..
        } => {
            remote_command_words(command)?;
            Some(GrantScope::RemoteCommand {
                plugin: plugin.clone(),
                target: target.clone(),
                label: label.clone(),
                endpoint: endpoint.clone(),
                command: command.clone(),
                background: *background,
            })
        }
        Access::Program { .. } => {
            let run = RunKey::of(access)?;
            Some(GrantScope::Program {
                plugin: run.plugin.to_owned(),
                name: run.name.to_owned(),
                program: run.program.to_owned(),
                fixed: run.fixed.to_vec(),
                arguments: run.arguments.to_vec(),
                working_directory: run.working_directory.to_owned(),
                effects: run.effects,
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
                background,
            },
            GrantScope::Command {
                command: narrower_command,
                working_directory: narrower_directory,
                background: narrower_background,
            },
        ) => {
            command == narrower_command
                && is_same_path(Path::new(narrower_directory), Path::new(working_directory))
                && covers_mode(*background, *narrower_background)
        }
        (
            GrantScope::RemoteCommand {
                plugin,
                target,
                endpoint,
                command,
                background,
                ..
            },
            GrantScope::RemoteCommand {
                plugin: narrower_plugin,
                target: narrower_target,
                endpoint: narrower_endpoint,
                command: narrower_command,
                background: narrower_background,
                ..
            },
        ) => {
            plugin == narrower_plugin
                && target == narrower_target
                && endpoint == narrower_endpoint
                && command == narrower_command
                && covers_mode(*background, *narrower_background)
        }
        (
            GrantScope::Program {
                plugin,
                name,
                program,
                fixed,
                arguments,
                working_directory,
                effects,
            },
            GrantScope::Program {
                plugin: narrower_plugin,
                name: narrower_name,
                program: narrower_program,
                fixed: narrower_fixed,
                arguments: narrower_arguments,
                working_directory: narrower_directory,
                effects: narrower_effects,
            },
        ) => {
            plugin == narrower_plugin
                && name == narrower_name
                && program == narrower_program
                && fixed == narrower_fixed
                && arguments == narrower_arguments
                && is_same_path(Path::new(narrower_directory), Path::new(working_directory))
                && covers_effects(effects, narrower_effects)
        }
        _ => false,
    }
}

/// The rule that "Save rule" can make: for an operation that is one command
/// that a rule can cover, here or on a remote host, or one run of an
/// installed program.
fn rule_for(accesses: &[Access]) -> Option<RuleProposal> {
    match accesses {
        [
            Access::Command {
                command,
                working_directory,
                background,
                ..
            },
        ] => Some(RuleProposal {
            words: command_words(command)?,
            folder: working_directory.clone(),
            remote: None,
            program: None,
            background: *background,
        }),
        [
            Access::RemoteCommand {
                plugin,
                target,
                label,
                endpoint,
                command,
                background,
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
            program: None,
            background: *background,
        }),
        _ => program_rule_for(accesses),
    }
}

/// The rule from an operation with one run of an installed program, beside
/// the paths that it reads or changes, which keep their own policy: the
/// run's words and folder, and the keys of its effects (ADR-0038, decision
/// 6).
fn program_rule_for(accesses: &[Access]) -> Option<RuleProposal> {
    let mut runs = accesses
        .iter()
        .filter(|access| matches!(access, Access::Program { .. }));
    let run = RunKey::of(runs.next()?)?;
    let is_alone = runs.next().is_none();
    let is_beside_paths = accesses
        .iter()
        .all(|access| matches!(access, Access::Path { .. } | Access::Program { .. }));
    if !(is_alone && is_beside_paths) {
        return None;
    }
    let words = run.words();
    Some(RuleProposal {
        words,
        folder: run.working_directory.to_owned(),
        remote: None,
        background: false,
        program: Some(ProgramTarget {
            plugin: run.plugin.to_owned(),
            name: run.name.to_owned(),
            program: run.program.to_owned(),
            effects: run.effects,
        }),
    })
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
        /// Whether it also covers commands in the background (ADR-0041).
        background: bool,
    },
    /// A rule for commands on one remote host (ADR-0036).
    Remote {
        /// The host, which the rule is always limited to.
        target: RemoteTarget,
        /// The leading words a command must start with.
        words: Vec<String>,
        /// Whether it also covers commands in the background (ADR-0041).
        background: bool,
    },
    /// A rule for runs of one plugin's installed program, with the effects
    /// of the run that it was saved from (ADR-0038).
    Program {
        /// The program, with the effects that the rule covers.
        target: ProgramTarget,
        /// The leading words that a run's program name and arguments must
        /// start with.
        words: Vec<String>,
        /// The folder the working directory must lie in, or `None` for
        /// anywhere.
        folder: Option<String>,
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
    let folder = limit_to_folder.then(|| proposal.folder.clone());
    Ok(match (&proposal.remote, &proposal.program) {
        (Some(target), _) => SavedRule::Remote {
            target: target.clone(),
            words,
            background: proposal.background,
        },
        (None, Some(target)) => SavedRule::Program {
            target: target.clone(),
            words,
            folder,
        },
        (None, None) => SavedRule::Local {
            words,
            folder,
            background: proposal.background,
        },
    })
}

#[cfg(test)]
mod tests;
