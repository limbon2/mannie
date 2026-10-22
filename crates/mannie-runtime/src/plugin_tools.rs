//! The plugins' tools in the core's tool loop (ADR-0034, decision 6): the
//! check of their names and schemas when their plugin starts, the agents
//! that a tool's selector offers it to, and the adapter through which the
//! core runs the SDK's tool as one of its own. The adapter goes through the
//! same lifecycle as the core's tools (ADR-0013): the call is prepared, the
//! policy decides on its accesses, the call is journaled, and only then does
//! it execute, until it ends or the owner stops the reply.
//!
//! A tool with side effects returns the accesses on which the policy
//! decides, such as a command on a remote host (ADR-0036), a path or a run
//! of an installed program (ADR-0038); a tool that reads the owner's files
//! returns its reads. The adapter turns them into the core's accesses with
//! the plugin's ID, adds the path that the core found and the arguments that
//! the manifest fixes to a program's run, so a plugin never speaks for
//! another, and refuses what breaks the SDK's rules before anything runs.
//!
//! The adapter also decides where a tool is offered, from its offering and
//! the conversation's workspace, and keeps the data and the failure code of
//! a result for the call's card, which the model never reads (ADR-0039).
//!
//! A call whose authorized accesses hold a remote command in the background
//! gets a ticket in its execution, which lets it hand the core that one
//! command while it executes (ADR-0041, decision 11).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_plugin_sdk::contributions::{Contributions, ExtensionPoint};
use mannie_plugin_sdk::host::{ContributionRefusal, ID_TAKEN, INVALID_NAME, RESERVED_ARGUMENT};
use mannie_plugin_sdk::points::executors::Executors;
use mannie_plugin_sdk::points::tools::{
    AgentSelector, MAX_FAILURE_CODE_CHARS, MAX_TOOL_ARCHIVE_BYTES, MAX_TOOL_ARCHIVES,
    MAX_TOOL_DATA_BYTES, PathAccess, PluginTool, ProgramAccess, RemoteCommandAccess,
    Tool as SdkTool, ToolAccess, ToolArchive, ToolCall, ToolEffect, ToolExecution, ToolOffering,
    ToolSpec, Tools,
};
use mannie_tools::{
    ArchivedOutput, Availability, Done, Execution, FolderError, PermissionClass, PrepareError,
    Prepared, Tool, ToolContext, ToolRegistry, plugin_path_access, stopped, takes_activity,
    workspace_folder,
};
use mannie_types::ids::ArchiveId;
use mannie_types::tools::{
    Access, AccessMode, MAX_ARGUMENTS_BYTES, MAX_EFFECT_ITEMS, ProgramEffect, ToolFailure,
    ToolOutcome,
};

use crate::Runtime;
use crate::agents::{AgentDefinition, EXPLORE_AGENT, Requester};
use crate::plugins::Snapshot;
use crate::tasks::TASK_TOOLS;
use crate::texts;

/// The most characters of a tool's name. The providers' interfaces accept
/// names of lower-case letters, digits and underscores up to this length.
const MAX_TOOL_NAME_CHARS: usize = 64;

/// The most accesses that one call of a plugin's tool may return.
const MAX_ACCESSES: usize = 16;

/// The most characters of a remote host's key, its name, its endpoint and
/// its identity, and of an archive's name.
const MAX_TARGET_CHARS: usize = 128;
const MAX_LABEL_CHARS: usize = 128;
const MAX_ENDPOINT_CHARS: usize = 512;
const MAX_IDENTITY_CHARS: usize = 512;
const MAX_ARCHIVE_NAME_CHARS: usize = 64;

/// The most characters of the state that a plugin read of a path.
const MAX_STATE_CHARS: usize = 1024;

/// The most arguments of a run of an installed program, and their most bytes
/// in all, which the longest command line of Windows holds.
const MAX_PROGRAM_ARGUMENTS: usize = 256;
const MAX_PROGRAM_ARGUMENT_BYTES: usize = 32 * 1024;

/// The most effects of one run, and the most characters of an effect's
/// texts, such as a hook's source or a discarded file.
const MAX_EFFECTS: usize = 32;
const MAX_EFFECT_TEXT_CHARS: usize = 1024;

/// The most characters of the entry whose presence offers a tool.
const MAX_MARKER_CHARS: usize = 64;

/// How many folders, the workspace included, the search for a marker
/// climbs. A repository's root is rarely farther above a project's folder.
const MAX_MARKER_DEPTH: usize = 32;

/// The failure code that stands in for a code that breaks the SDK's rules,
/// so the call's card still shows that the call failed.
const FAILED: &str = "failed";

/// A plugin's tool that a run is offered, with the start of its plugin that
/// offered it, so a call runs only while that start is on (ADR-0030,
/// decision 8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OfferedPluginTool {
    /// The tool's name.
    pub(crate) name: String,
    /// The plugin's ID.
    pub(crate) plugin: String,
    /// The generation of the plugin's start.
    pub(crate) generation: u64,
}

/// The plugins' tools of a snapshot that runs of `agent` are offered, those
/// whose selectors name the agent, each with the adapter that the tool loop
/// runs. The adapters find the installed programs of `runtime`.
pub(crate) fn offered_tools(
    snapshot: &Snapshot,
    agent: &AgentDefinition,
    runtime: &Runtime,
) -> Vec<(OfferedPluginTool, Arc<dyn Tool>)> {
    snapshot
        .items::<Tools>()
        .iter()
        .filter(|registered| offers(&registered.entry, agent))
        .map(|registered| {
            let plugin = registered.plugin.as_str().to_owned();
            let offered = OfferedPluginTool {
                name: registered.entry.spec.name.clone(),
                plugin: plugin.clone(),
                generation: registered.generation,
            };
            let adapter: Arc<dyn Tool> = Arc::new(PluginToolAdapter {
                plugin,
                tool: Arc::clone(&registered.entry.tool),
                spec: registered.entry.spec.clone(),
                effect: registered.entry.effect,
                offering: registered.entry.offering.clone(),
                markers: Mutex::new(HashMap::new()),
                runtime: runtime.clone(),
            });
            (offered, adapter)
        })
        .collect()
}

/// Whether a plugin's tool is offered to the runs of `agent`: its selector
/// names the agent, and a tool with side effects never reaches the explore
/// agent, which only reads, whatever the selector says.
fn offers(entry: &PluginTool, agent: &AgentDefinition) -> bool {
    let reads_only = agent.name == EXPLORE_AGENT;
    let acts = entry.effect == ToolEffect::SideEffects;
    offered_to(&entry.offered_to, agent) && !(reads_only && acts)
}

/// Whether a tool's selector offers it to the runs of `agent`.
pub(crate) fn offered_to(selector: &AgentSelector, agent: &AgentDefinition) -> bool {
    match selector {
        AgentSelector::Conversations => agent.requester == Requester::Owner,
        AgentSelector::ChildAgents => agent.requester == Requester::Agent,
        AgentSelector::Every => true,
        AgentSelector::Agents(names) => names.iter().any(|name| name == agent.name),
    }
}

impl Runtime {
    /// Refuses a plugin's tools whose names the core, its task tools,
    /// another plugin or an executor has, the registered executors and the
    /// plugin's own (the rule `Global`), and the tools that
    /// [`check_plugin_tools`] refuses for their names and schemas.
    pub(crate) fn check_tools(
        &self,
        snapshot: &Snapshot,
        contributions: &Contributions,
    ) -> Result<(), ContributionRefusal> {
        let plugin_tools: Vec<&str> = snapshot
            .items::<Tools>()
            .iter()
            .map(|registered| registered.entry.spec.name.as_str())
            .collect();
        // An executor's tool joins the task tools of a run, so a plugin's
        // tool of its name would give the model two definitions of one name.
        // An executor that offers no tool now may offer one later, which the
        // run's setup checks again.
        let executor_tools: Vec<String> = snapshot
            .items::<Executors>()
            .iter()
            .map(|registered| &registered.entry)
            .chain(contributions.entries::<Executors>())
            .filter_map(|executor| executor.tool())
            .map(|spec| spec.name)
            .collect();
        let executor_tools: Vec<&str> = executor_tools.iter().map(String::as_str).collect();
        check_plugin_tools(
            &self.inner.tools.registry,
            &plugin_tools,
            &executor_tools,
            contributions.entries::<Tools>(),
        )
    }
}

/// Refuses a plugin's tools before any of them is registered: a name that is
/// not lower-case snake case of at most 64 characters, a name of the core's
/// tools or task tools, of `plugin_tools` (the tools of the plugins that are
/// on), of `executor_tools` (the tools that the executors offer, the
/// plugin's own executor included) or of another of the plugin's own tools
/// (the rule `Global`), and a schema with the argument that every call
/// carries for the owner, which the registry refuses for the core's tools.
pub(crate) fn check_plugin_tools<'a>(
    registry: &ToolRegistry,
    plugin_tools: &[&str],
    executor_tools: &[&str],
    tools: impl IntoIterator<Item = &'a PluginTool>,
) -> Result<(), ContributionRefusal> {
    let mut names: Vec<&str> = Vec::new();
    for entry in tools {
        let name = entry.spec.name.as_str();
        if !is_valid_tool_name(name) {
            return Err(ContributionRefusal::new(
                Tools::NAME,
                INVALID_NAME,
                format!(
                    "the tool name {name:?} is not lower-case snake case of at most 64 characters"
                ),
            ));
        }
        let taken = registry.get(name).is_some()
            || TASK_TOOLS.contains(&name)
            || plugin_tools.contains(&name)
            || executor_tools.contains(&name)
            || names.contains(&name);
        if taken {
            return Err(ContributionRefusal::new(
                Tools::NAME,
                ID_TAKEN,
                format!("a tool named {name} is registered already"),
            ));
        }
        if takes_activity(&entry.spec.parameters) {
            return Err(ContributionRefusal::new(
                Tools::NAME,
                RESERVED_ARGUMENT,
                format!(
                    "the tool {name} has an argument named activity, which every call carries for the owner"
                ),
            ));
        }
        if let ToolOffering::WhereWorkspaceHolds(marker) = &entry.offering
            && !is_valid_marker(marker)
        {
            return Err(ContributionRefusal::new(
                Tools::NAME,
                INVALID_NAME,
                format!(
                    "the tool {name} is offered where the workspace holds {marker:?}, which is not the name of one entry"
                ),
            ));
        }
        names.push(name);
    }
    Ok(())
}

/// Whether a marker names one entry of a folder: 1 to 64 characters without
/// a separator, a colon or a control character, and neither `.` nor `..`, so
/// the search for it never leaves the folders that it climbs.
fn is_valid_marker(marker: &str) -> bool {
    !marker.is_empty()
        && marker.chars().count() <= MAX_MARKER_CHARS
        && marker != "."
        && marker != ".."
        && !marker
            .chars()
            .any(|character| matches!(character, '/' | '\\' | ':') || character.is_control())
}

/// Whether a tool with `offering` is offered where `availability` says: a
/// conversation's workspace is its project's folder (ADR-0038, decision 2).
/// `markers` keeps what a run found for each workspace.
fn is_offered(
    offering: &ToolOffering,
    markers: &Mutex<HashMap<String, bool>>,
    availability: &Availability<'_>,
) -> bool {
    match offering {
        ToolOffering::Always => true,
        ToolOffering::InWorkspace => availability.project.is_some(),
        ToolOffering::WhereWorkspaceHolds(marker) => {
            let Some(project) = availability.project else {
                return false;
            };
            let mut markers = markers.lock().unwrap_or_else(PoisonError::into_inner);
            *markers
                .entry(project.folder.clone())
                .or_insert_with(|| holds_marker(Path::new(&project.folder), marker))
        }
    }
}

/// Whether `workspace`, or one of the folders above it, holds an entry named
/// `marker`. Links are not followed: a `.git` that is a link counts as
/// present, and nothing outside the folders is read.
fn holds_marker(workspace: &Path, marker: &str) -> bool {
    workspace
        .ancestors()
        .take(MAX_MARKER_DEPTH)
        .any(|folder| std::fs::symlink_metadata(folder.join(marker)).is_ok())
}

/// Whether a failure code keeps the SDK's rules: 1 to 64 lower-case letters
/// and underscores.
fn is_valid_code(code: &str) -> bool {
    !code.is_empty()
        && code.chars().count() <= MAX_FAILURE_CODE_CHARS
        && code
            .chars()
            .all(|character| character.is_ascii_lowercase() || character == '_')
}

/// Whether a name starts with a lower-case letter and goes on with
/// lower-case letters, digits and underscores, up to 64 characters.
fn is_valid_tool_name(name: &str) -> bool {
    let mut characters = name.chars();
    let starts_with_letter = characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase());
    starts_with_letter
        && name.len() <= MAX_TOOL_NAME_CHARS
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

/// An output within `limit` bytes. A longer one is cut at a character and
/// ends with a note that tells the model so.
fn within_limit(mut output: String, limit: usize) -> String {
    if output.len() <= limit {
        return output;
    }
    let note = texts::plugin_output_cut(output.len());
    // The cut output, a line break and the note fit in the limit.
    let end = output.floor_char_boundary(limit.saturating_sub(note.len() + 1));
    output.truncate(end);
    output.push('\n');
    output.push_str(&note);
    output
}

/// A plugin's tool as the core's tool loop runs it.
#[derive(Debug)]
struct PluginToolAdapter {
    /// The plugin's ID, which the call's outcome and accesses name.
    plugin: String,
    /// The plugin's tool.
    tool: Arc<dyn SdkTool>,
    /// What the model is told of the tool, as the plugin's entry read it.
    spec: ToolSpec,
    /// What the tool does beyond reading its plugin's own data.
    effect: ToolEffect,
    /// Where the tool is offered.
    offering: ToolOffering,
    /// Whether each workspace holds the offering's marker, as this run found
    /// it. The run asks for every model call, so the file system is asked
    /// once for each workspace.
    markers: Mutex<HashMap<String, bool>>,
    /// The runtime whose installed programs a program access names.
    runtime: Runtime,
}

impl Tool for PluginToolAdapter {
    fn name(&self) -> &str {
        &self.spec.name
    }

    fn description(&self) -> &str {
        &self.spec.description
    }

    fn parameters(&self) -> &serde_json::Value {
        &self.spec.parameters
    }

    /// A tool that reads only its plugin's own data, or the owner's files,
    /// never performs an operation (ADR-0034, decision 6; ADR-0038); a tool
    /// with side effects performs one in every call, which the journal
    /// records (ADR-0036).
    fn class(&self) -> PermissionClass {
        match self.effect {
            ToolEffect::ReadsOwnData | ToolEffect::ReadsFiles => PermissionClass::ReadOnly,
            ToolEffect::SideEffects => PermissionClass::SideEffects,
        }
    }

    fn offered(&self, availability: &Availability<'_>) -> bool {
        is_offered(&self.offering, &self.markers, availability)
    }

    fn prepare(
        &self,
        context: ToolContext,
        arguments: String,
    ) -> BoxFuture<'static, Result<Prepared, PrepareError>> {
        let preparing = self.tool.prepare(
            ToolCall::new(arguments, context.conversation_id).with_workspace(context.workspace),
        );
        let plugin = self.plugin.clone();
        let effect = self.effect;
        let limit = context.result_limit;
        let runtime = self.runtime.clone();
        async move {
            let accesses = match preparing.await {
                Ok(accesses) => accesses,
                Err(refusal) => return Err(refused(&plugin, refusal.detail, refusal.code, limit)),
            };
            let programs = resolved_programs(&runtime, &plugin, &accesses).await?;
            // Paths are resolved against the file system, which blocks.
            tokio::task::spawn_blocking(move || {
                core_accesses(&plugin, effect, accesses, &programs)
                    .map(|accesses| Prepared { accesses })
            })
            .await
            .unwrap_or_else(|_join_error| {
                Err(PrepareError::Failed(ToolFailure::IoError {
                    kind: "the preparation of the call was cut off".to_owned(),
                }))
            })
        }
        .boxed()
    }

    fn execute(&self, execution: Execution) -> BoxFuture<'static, Done> {
        let Execution {
            context,
            arguments,
            accesses,
            stop,
            part_id,
            ..
        } = execution;
        let limit = context.result_limit;
        let reserved: Vec<ArchiveId> = (0..MAX_TOOL_ARCHIVES)
            .map(|_| (context.archive_ids)())
            .collect();
        let ticket = part_id.and_then(|part_id| {
            self.runtime.issue_background_ticket(
                &self.plugin,
                &context.conversation_id,
                &part_id,
                &accesses,
            )
        });
        let mut tool_execution = ToolExecution::new(arguments, context.conversation_id)
            .with_authorized(sdk_accesses(&accesses))
            .with_result_limit(u32::try_from(limit).unwrap_or(u32::MAX))
            .with_archive_ids(reserved.clone())
            .with_workspace(context.workspace);
        if let Some(guard) = &ticket {
            tool_execution = tool_execution.with_background(guard.ticket());
        }
        let running = self.tool.execute(tool_execution);
        let plugin = self.plugin.clone();
        async move {
            // The ticket is good while the call executes.
            let _ticket = ticket;
            tokio::select! {
                // A result that is ready wins over a stop that arrives at the
                // same moment, so an operation that finished keeps its result.
                biased;
                result = running => {
                    let archives = kept_archives(&plugin, &reserved, result.archives);
                    let data = card_data(&plugin, result.data);
                    let failure = result.failure.map(|code| kept_code(&plugin, code));
                    Done {
                        outcome: ToolOutcome::Plugin { plugin, data, failure },
                        output: within_limit(result.output, limit),
                        archives,
                    }
                }
                // The SDK cancels a call by dropping its future (ADR-0034,
                // decision 3), so Stop and Shutdown end the plugin's work
                // here, and the run waits for nothing that may never end. A
                // run makes such a call with side effects uncertain.
                () = stop.stopped() => Done::failed(stopped()),
            }
        }
        .boxed()
    }
}

/// What the core adds to a run of an installed program: the program's
/// canonical path and the arguments that the plugin's manifest fixes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedProgram {
    path: String,
    fixed: Vec<String>,
}

/// The programs that a call's program accesses name, as the plugin's
/// manifest declares them and the core found them. A program that the
/// manifest does not declare breaks the SDK's rules, and one that the core
/// cannot find fails the call with what the model reads.
async fn resolved_programs(
    runtime: &Runtime,
    plugin: &str,
    accesses: &[ToolAccess],
) -> Result<HashMap<String, ResolvedProgram>, PrepareError> {
    let mut resolved = HashMap::new();
    for access in accesses {
        let ToolAccess::Program(run) = access else {
            continue;
        };
        if resolved.contains_key(&run.name) {
            continue;
        }
        let Some(section) = runtime.inner.programs.declared(plugin, &run.name) else {
            return Err(broken(plugin));
        };
        let not_found = || {
            PrepareError::Failed(ToolFailure::SpawnFailed {
                reason: format!("{} was not found on the PATH", run.name),
            })
        };
        let path = runtime
            .locate_program(&run.name)
            .await
            .map_err(|_not_found| not_found())?;
        let path = path.to_str().ok_or_else(not_found)?.to_owned();
        resolved.insert(
            run.name.clone(),
            ResolvedProgram {
                path,
                fixed: section.fixed,
            },
        );
    }
    Ok(resolved)
}

/// The core's accesses of a call of a plugin's tool, or the failure of a
/// plugin that broke the SDK's rules: a tool that reads its own data returns
/// no access, a tool that reads files returns 1 to 16 reads of paths, a tool
/// with side effects returns 1 to 16 accesses, and every access keeps its
/// limits (ADR-0036, ADR-0038). Nothing runs after such a failure. A path
/// that the core refuses, such as a UNC path, fails the call with the
/// refusal, which the model reads. It resolves paths, so it blocks.
fn core_accesses(
    plugin: &str,
    effect: ToolEffect,
    accesses: Vec<ToolAccess>,
    programs: &HashMap<String, ResolvedProgram>,
) -> Result<Vec<Access>, PrepareError> {
    let fits = match effect {
        ToolEffect::ReadsOwnData => accesses.is_empty(),
        ToolEffect::ReadsFiles => !accesses.is_empty()
            && accesses.len() <= MAX_ACCESSES
            && accesses.iter().all(
                |access| matches!(access, ToolAccess::Path(path) if path.mode == AccessMode::Read),
            ),
        ToolEffect::SideEffects => !accesses.is_empty() && accesses.len() <= MAX_ACCESSES,
    };
    // A call hands the core at most one command in the background, the one
    // that its ticket names.
    let in_background = accesses
        .iter()
        .filter(|access| matches!(access, ToolAccess::RemoteCommand(remote) if remote.background))
        .count();
    if !fits || in_background > 1 {
        return Err(broken(plugin));
    }
    accesses
        .into_iter()
        .map(|access| match access {
            ToolAccess::RemoteCommand(remote) => {
                remote_access(plugin, remote).ok_or_else(|| broken(plugin))
            }
            ToolAccess::Path(path) => path_access(plugin, path),
            ToolAccess::Program(run) => program_access(plugin, run, programs),
        })
        .collect()
}

/// The core's access of a run of an installed program, with the plugin, the
/// program's path and the fixed arguments that the core adds, in the
/// canonical working directory. Arguments past their limits or with NUL,
/// more than 32 effects, an effect past its limits, a state that is empty,
/// too long or holds NUL, and a working directory that is not absolute or
/// not Unicode break the SDK's rules. A working directory that the core
/// refuses, such as a UNC path, or that is no folder, fails the call.
fn program_access(
    plugin: &str,
    run: ProgramAccess,
    programs: &HashMap<String, ResolvedProgram>,
) -> Result<Access, PrepareError> {
    let Some(resolved) = programs.get(&run.name) else {
        return Err(broken(plugin));
    };
    let argument_bytes: usize = run.arguments.iter().map(String::len).sum();
    let arguments_fit = run.arguments.len() <= MAX_PROGRAM_ARGUMENTS
        && argument_bytes <= MAX_PROGRAM_ARGUMENT_BYTES
        && !run.arguments.iter().any(|argument| argument.contains('\0'));
    let effects_fit = run.effects.len() <= MAX_EFFECTS && run.effects.iter().all(effect_fits);
    let state_fits = run.state.as_ref().is_none_or(|state| {
        !state.is_empty() && state.chars().count() <= MAX_STATE_CHARS && !state.contains('\0')
    });
    let Some(directory) = run.working_directory.to_str() else {
        return Err(broken(plugin));
    };
    let fits = arguments_fit && effects_fit && state_fits && run.working_directory.is_absolute();
    if !fits {
        return Err(broken(plugin));
    }
    let working_directory =
        workspace_folder(directory).map_err(|error| folder_failure(directory, error))?;
    Ok(Access::Program {
        plugin: plugin.to_owned(),
        name: run.name,
        program: resolved.path.clone(),
        fixed: resolved.fixed.clone(),
        arguments: run.arguments,
        working_directory,
        effects: run.effects,
        state: run.state,
    })
}

/// The failure of a call whose program's working directory the core refused
/// or could not use.
fn folder_failure(directory: &str, error: FolderError) -> PrepareError {
    let path = directory.to_owned();
    PrepareError::Failed(match error {
        FolderError::Refused { reason } => ToolFailure::PathRefused { reason },
        FolderError::NotFound => ToolFailure::NotFound { path },
        FolderError::NotAFolder => ToolFailure::NotAFolder { path },
        FolderError::Unreadable { kind } => ToolFailure::IoError { kind },
    })
}

/// Whether an effect keeps its limits: texts that are not empty, within
/// their lengths and without control characters, at most
/// [`MAX_EFFECT_ITEMS`] items or places with a count of at least as many,
/// and an endpoint without a password, which never reaches an approval or
/// the journal (T-47).
fn effect_fits(effect: &ProgramEffect) -> bool {
    let fits = |text: &str, max_chars: usize| {
        !text.is_empty() && text.chars().count() <= max_chars && !text.chars().any(char::is_control)
    };
    let list_fits = |count: u32, items: &[String]| {
        items.len() <= MAX_EFFECT_ITEMS
            && usize::try_from(count).is_ok_and(|count| count >= items.len())
            && items.iter().all(|item| fits(item, MAX_EFFECT_TEXT_CHARS))
    };
    match effect {
        ProgramEffect::Network { endpoint } => {
            fits(endpoint, MAX_ENDPOINT_CHARS) && !holds_password(endpoint)
        }
        ProgramEffect::RunsFolderCode { name, source } => {
            fits(name, MAX_LABEL_CHARS) && fits(source, MAX_EFFECT_TEXT_CHARS)
        }
        ProgramEffect::DiscardsWork { count, items, .. } => list_fits(*count, items),
        ProgramEffect::RewritesPublished { count, places } => list_fits(*count, places),
    }
}

/// Whether a URL names a password before its host, as in
/// `https://user:token@host/`. A user alone, as in `ssh://git@host/`, is no
/// secret.
fn holds_password(endpoint: &str) -> bool {
    let Some((_scheme, rest)) = endpoint.split_once("://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    authority
        .rsplit_once('@')
        .is_some_and(|(user_information, _host)| user_information.contains(':'))
}

/// The failure of a call whose plugin broke the SDK's rules, which the log
/// records.
fn broken(plugin: &str) -> PrepareError {
    tracing::error!(
        plugin,
        "a plugin's tool returned accesses that break the SDK's rules"
    );
    PrepareError::Failed(ToolFailure::PluginInvalid {
        plugin: plugin.to_owned(),
    })
}

/// The core's access of a path, in its canonical place, with the state that
/// the plugin read of it. A path that is not absolute or not Unicode, and a
/// state that is empty, too long or holds NUL, break the SDK's rules.
fn path_access(plugin: &str, path: PathAccess) -> Result<Access, PrepareError> {
    let state_fits = path.state.as_ref().is_none_or(|state| {
        !state.is_empty() && state.chars().count() <= MAX_STATE_CHARS && !state.contains('\0')
    });
    let Some(text) = path.path.to_str() else {
        return Err(broken(plugin));
    };
    if !state_fits || !path.path.is_absolute() {
        return Err(broken(plugin));
    }
    plugin_path_access(text, path.mode, path.state).map_err(PrepareError::Failed)
}

/// The end of a call whose arguments a plugin refused. A refusal with a code
/// ends the call completed with that failure, for the call's card, and the
/// model reads the detail as the result (ADR-0039); any other refusal keeps
/// the call from running for its arguments.
fn refused(plugin: &str, detail: String, code: Option<String>, limit: usize) -> PrepareError {
    match code {
        Some(code) if is_valid_code(&code) => PrepareError::Ended(Done {
            outcome: ToolOutcome::Plugin {
                plugin: plugin.to_owned(),
                data: None,
                failure: Some(code),
            },
            output: within_limit(detail, limit),
            archives: Vec::new(),
        }),
        Some(_) => {
            tracing::error!(
                plugin,
                "a plugin's refusal carries a code that breaks the SDK's rules"
            );
            PrepareError::InvalidArguments { detail }
        }
        None => PrepareError::InvalidArguments { detail },
    }
}

/// The data of a call's card as JSON text, when it is an object within the
/// limit; other data is left out, which the log records, so broken data
/// never costs a call its result.
fn card_data(plugin: &str, data: Option<serde_json::Value>) -> Option<String> {
    let data = data?;
    let text = data
        .is_object()
        .then(|| serde_json::to_string(&data).ok())
        .flatten();
    match text {
        Some(text) if text.len() <= MAX_TOOL_DATA_BYTES => Some(text),
        _ => {
            tracing::error!(
                plugin,
                "left out the data of a plugin's card that breaks the SDK's rules"
            );
            None
        }
    }
}

/// A result's failure code, or [`FAILED`] in place of one that breaks the
/// SDK's rules, so the card still shows that the call failed.
fn kept_code(plugin: &str, code: String) -> String {
    if is_valid_code(&code) {
        code
    } else {
        tracing::error!(
            plugin,
            "a plugin's result carries a failure code that breaks the SDK's rules"
        );
        FAILED.to_owned()
    }
}

/// The core's access of a command on a remote host, with the plugin's ID,
/// or `None` when a text is empty, too long or holds a control character.
/// The command may span lines, which only keeps it from a grant or a rule.
fn remote_access(plugin: &str, remote: RemoteCommandAccess) -> Option<Access> {
    let fits = |text: &str, max_chars: usize| {
        !text.is_empty() && text.chars().count() <= max_chars && !text.chars().any(char::is_control)
    };
    let command_fits = !remote.command.trim().is_empty()
        && remote.command.len() <= MAX_ARGUMENTS_BYTES
        && !remote.command.contains('\0');
    let names_fit = fits(&remote.target, MAX_TARGET_CHARS)
        && fits(&remote.label, MAX_LABEL_CHARS)
        && fits(&remote.endpoint, MAX_ENDPOINT_CHARS)
        && fits(&remote.identity, MAX_IDENTITY_CHARS);
    (command_fits && names_fit).then(|| Access::RemoteCommand {
        plugin: plugin.to_owned(),
        target: remote.target,
        label: remote.label,
        endpoint: remote.endpoint,
        identity: remote.identity,
        command: remote.command,
        background: remote.background,
    })
}

/// The authorized accesses of a call as its plugin's tool knows them.
fn sdk_accesses(accesses: &[Access]) -> Vec<ToolAccess> {
    accesses
        .iter()
        .filter_map(|access| match access {
            Access::RemoteCommand {
                target,
                label,
                endpoint,
                identity,
                command,
                background,
                ..
            } => {
                let access = RemoteCommandAccess::new(
                    target.clone(),
                    label.clone(),
                    endpoint.clone(),
                    identity.clone(),
                    command.clone(),
                );
                Some(ToolAccess::RemoteCommand(if *background {
                    access.in_background()
                } else {
                    access
                }))
            }
            Access::Path {
                path, mode, state, ..
            } => {
                let access = PathAccess::new(path.clone(), *mode);
                Some(ToolAccess::Path(match state {
                    Some(state) => access.with_state(state.clone()),
                    None => access,
                }))
            }
            Access::Program {
                name,
                arguments,
                working_directory,
                effects,
                state,
                ..
            } => {
                let mut access =
                    ProgramAccess::new(name.clone(), working_directory.clone(), arguments.clone());
                for effect in effects {
                    access = access.with_effect(effect.clone());
                }
                Some(ToolAccess::Program(match state {
                    Some(state) => access.with_state(state.clone()),
                    None => access,
                }))
            }
            Access::Command { .. } | Access::Project { .. } => None,
        })
        .collect()
}

/// The archives of a plugin's result that keep the SDK's rules: each under
/// an ID that the execution reserved, once, with a name and within the size
/// limit. The rest are left out, which the log records, so a broken archive
/// never costs a call that ran its result.
fn kept_archives(
    plugin: &str,
    reserved: &[ArchiveId],
    archives: Vec<ToolArchive>,
) -> Vec<ArchivedOutput> {
    let mut used: HashSet<ArchiveId> = HashSet::new();
    let mut kept = Vec::new();
    for archive in archives {
        let name_fits = !archive.name.is_empty()
            && archive.name.chars().count() <= MAX_ARCHIVE_NAME_CHARS
            && !archive.name.chars().any(char::is_control);
        let fits = reserved.contains(&archive.id)
            && !used.contains(&archive.id)
            && name_fits
            && archive.content.len() <= MAX_TOOL_ARCHIVE_BYTES;
        if !fits {
            tracing::error!(
                plugin,
                "left out an archive of a plugin's tool that breaks the SDK's rules"
            );
            continue;
        }
        used.insert(archive.id.clone());
        kept.push(ArchivedOutput {
            id: archive.id,
            name: archive.name,
            content: archive.content,
        });
    }
    kept
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use futures_util::FutureExt;
    use mannie_plugin_sdk::BoxFuture;
    use mannie_plugin_sdk::points::tools::{
        AgentSelector, MAX_TOOL_ARCHIVE_BYTES, MAX_TOOL_DATA_BYTES, PathAccess, PluginTool,
        ProgramAccess, RemoteCommandAccess, Tool, ToolAccess, ToolArchive, ToolCall, ToolEffect,
        ToolExecution, ToolOffering, ToolRefusal, ToolResult, ToolSpec,
    };
    use mannie_tools::{Availability, Done, PrepareError, ToolRegistry};
    use mannie_types::clock::UtcMillis;
    use mannie_types::ids::{ArchiveId, ProjectId};
    use mannie_types::projects::ProjectView;
    use mannie_types::tools::{
        Access, AccessMode, DiscardedWork, ProgramEffect, ToolFailure, ToolOutcome,
    };
    use serde_json::{Value, json};

    use mannie_plugin_sdk::host::{ID_TAKEN, INVALID_NAME, RESERVED_ARGUMENT};

    use super::{
        MAX_TOOL_NAME_CHARS, ResolvedProgram, card_data, check_plugin_tools, holds_marker,
        holds_password, is_offered, is_valid_marker, is_valid_tool_name, kept_archives, kept_code,
        offered_to, offers, refused, sdk_accesses, within_limit,
    };
    use crate::agents::{AgentDefinition, CONVERSATION, EXPLORE, GENERAL};
    use crate::texts;

    /// A marker that no folder above a test's temporary folder holds.
    const MARKER: &str = "mannie-test-marker";

    /// The core's accesses of a call that runs no installed program.
    fn core_accesses(
        plugin: &str,
        effect: ToolEffect,
        accesses: Vec<ToolAccess>,
    ) -> Result<Vec<Access>, PrepareError> {
        super::core_accesses(plugin, effect, accesses, &HashMap::new())
    }

    fn read(path: &Path) -> ToolAccess {
        ToolAccess::Path(PathAccess::new(path, AccessMode::Read))
    }

    fn write(path: &Path) -> ToolAccess {
        ToolAccess::Path(PathAccess::new(path, AccessMode::Write))
    }

    fn canonical(path: &Path) -> String {
        mannie_tools::canonical_folder(path).unwrap()
    }

    #[test]
    fn a_tool_that_reads_files_returns_reads_of_paths_and_nothing_else() {
        let folder = tempfile::tempdir().unwrap();

        let reads = core_accesses("shell", ToolEffect::ReadsFiles, vec![read(folder.path())]);

        let expected = canonical(folder.path());
        assert!(
            matches!(
                reads.as_deref(),
                Ok([Access::Path {
                    path,
                    mode: AccessMode::Read,
                    folder: true,
                    exists: true,
                    state: None,
                    ..
                }]) if *path == expected
            ),
            "{reads:?}"
        );
        let broken = [
            Vec::new(),
            vec![write(folder.path())],
            vec![remote("key-1", "box", "uptime")],
            (0..17).map(|_| read(folder.path())).collect(),
        ];
        for accesses in broken {
            assert_eq!(
                core_accesses("shell", ToolEffect::ReadsFiles, accesses.clone()),
                Err(invalid()),
                "{accesses:?}"
            );
        }
    }

    #[test]
    fn a_path_keeps_its_state_and_a_relative_path_or_a_broken_state_breaks_the_rules() {
        let folder = tempfile::tempdir().unwrap();
        let pinned = ToolAccess::Path(
            PathAccess::new(folder.path(), AccessMode::Write).with_state("HEAD 1a2b"),
        );

        let accesses = core_accesses("shell", ToolEffect::SideEffects, vec![pinned]).unwrap();

        assert!(
            matches!(
                accesses.as_slice(),
                [Access::Path { mode: AccessMode::Write, state: Some(state), .. }] if state == "HEAD 1a2b"
            ),
            "{accesses:?}"
        );
        let broken = [
            ToolAccess::Path(PathAccess::new("relative/folder", AccessMode::Read)),
            ToolAccess::Path(PathAccess::new(folder.path(), AccessMode::Read).with_state("")),
            ToolAccess::Path(
                PathAccess::new(folder.path(), AccessMode::Read).with_state("x".repeat(1025)),
            ),
            ToolAccess::Path(PathAccess::new(folder.path(), AccessMode::Read).with_state("a\0b")),
        ];
        for access in broken {
            assert_eq!(
                core_accesses("shell", ToolEffect::SideEffects, vec![access.clone()]),
                Err(invalid()),
                "{access:?}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_unc_path_is_refused_before_any_access_to_the_file_system() {
        let refused = core_accesses(
            "shell",
            ToolEffect::ReadsFiles,
            vec![read(Path::new(r"\\server\share\repository"))],
        );

        assert!(
            matches!(
                refused,
                Err(PrepareError::Failed(ToolFailure::PathRefused { .. }))
            ),
            "{refused:?}"
        );
    }

    #[test]
    fn authorized_paths_return_to_the_plugin_with_their_state() {
        let accesses = [Access::Path {
            path: "/work/repository".to_owned(),
            mode: AccessMode::Write,
            folder: true,
            exists: true,
            identity: "1:1".to_owned(),
            entries: None,
            state: Some("HEAD 1a".to_owned()),
        }];

        assert_eq!(
            sdk_accesses(&accesses),
            vec![ToolAccess::Path(
                PathAccess::new("/work/repository", AccessMode::Write).with_state("HEAD 1a")
            )]
        );
    }

    /// The installed git as the core found it, with one fixed setting.
    fn found_git() -> HashMap<String, ResolvedProgram> {
        HashMap::from([(
            "git".to_owned(),
            ResolvedProgram {
                path: "/usr/bin/git".to_owned(),
                fixed: vec!["-c".to_owned(), "gc.auto=0".to_owned()],
            },
        )])
    }

    fn hook() -> ProgramEffect {
        ProgramEffect::RunsFolderCode {
            name: "pre-commit".to_owned(),
            source: ".git/hooks/pre-commit".to_owned(),
        }
    }

    fn git_run(folder: &Path, arguments: &[&str]) -> ProgramAccess {
        let arguments = arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect();
        ProgramAccess::new("git", folder, arguments)
    }

    #[test]
    fn a_program_run_gets_the_path_and_the_fixed_arguments_that_the_core_adds() {
        let folder = tempfile::tempdir().unwrap();
        let run = git_run(folder.path(), &["commit", "-F", "-"])
            .with_effect(hook())
            .with_state("HEAD 1a2b");

        let accesses = super::core_accesses(
            "git",
            ToolEffect::SideEffects,
            vec![ToolAccess::Program(run), write(folder.path())],
            &found_git(),
        )
        .unwrap();

        assert_eq!(
            accesses[0],
            Access::Program {
                plugin: "git".to_owned(),
                name: "git".to_owned(),
                program: "/usr/bin/git".to_owned(),
                fixed: vec!["-c".to_owned(), "gc.auto=0".to_owned()],
                arguments: vec!["commit".to_owned(), "-F".to_owned(), "-".to_owned()],
                working_directory: canonical(folder.path()),
                effects: vec![hook()],
                state: Some("HEAD 1a2b".to_owned()),
            }
        );
        assert!(matches!(accesses[1], Access::Path { .. }), "{accesses:?}");
        // The plugin learns the run as the core authorized it.
        assert_eq!(
            sdk_accesses(&accesses[..1]),
            vec![ToolAccess::Program(
                git_run(Path::new(&canonical(folder.path())), &["commit", "-F", "-"])
                    .with_effect(hook())
                    .with_state("HEAD 1a2b")
            )]
        );
    }

    #[test]
    fn a_program_run_that_breaks_the_sdk_s_rules_is_refused() {
        let folder = tempfile::tempdir().unwrap();
        let run = |arguments: &[&str]| git_run(folder.path(), arguments);
        let discards = |count: u32, items: usize| ProgramEffect::DiscardsWork {
            work: DiscardedWork::UncommittedChanges,
            count,
            items: (0..items).map(|item| format!("file-{item}")).collect(),
            snapshot: true,
        };
        let network = |endpoint: &str| ProgramEffect::Network {
            endpoint: endpoint.to_owned(),
        };
        let many: Vec<String> = (0..257).map(|number| number.to_string()).collect();
        let long = "x".repeat(32 * 1024 + 1);
        let broken = [
            ProgramAccess::new("pandoc", folder.path(), Vec::new()),
            run(&["commit", "a\0b"]),
            ProgramAccess::new("git", folder.path(), many),
            run(&[long.as_str()]),
            git_run(Path::new("relative"), &["status"]),
            run(&["status"]).with_state(""),
            run(&["push"]).with_effect(network("")),
            run(&["push"]).with_effect(network("https://user:token@example.com/repo.git")),
            run(&["reset"]).with_effect(discards(21, 21)),
            run(&["reset"]).with_effect(discards(1, 2)),
        ];
        for access in broken {
            assert_eq!(
                super::core_accesses(
                    "shell",
                    ToolEffect::SideEffects,
                    vec![ToolAccess::Program(access.clone())],
                    &found_git()
                ),
                Err(invalid()),
                "{access:?}"
            );
        }
        let mut too_many = run(&["commit"]);
        for _ in 0..33 {
            too_many = too_many.with_effect(hook());
        }
        assert_eq!(
            super::core_accesses(
                "shell",
                ToolEffect::SideEffects,
                vec![ToolAccess::Program(too_many)],
                &found_git()
            ),
            Err(invalid())
        );
        // A tool that only reads files never runs a program.
        assert_eq!(
            super::core_accesses(
                "shell",
                ToolEffect::ReadsFiles,
                vec![ToolAccess::Program(run(&["status"]))],
                &found_git()
            ),
            Err(invalid())
        );
        let missing = super::core_accesses(
            "shell",
            ToolEffect::SideEffects,
            vec![ToolAccess::Program(git_run(
                &folder.path().join("missing"),
                &["status"],
            ))],
            &found_git(),
        );
        assert!(
            matches!(
                missing,
                Err(PrepareError::Failed(ToolFailure::NotFound { .. }))
            ),
            "{missing:?}"
        );
    }

    #[test]
    fn only_a_password_before_the_host_counts_as_one() {
        for endpoint in [
            "https://user:token@example.com/repo.git",
            "https://:token@example.com",
            "ssh://git:secret@example.com:22/repo.git",
        ] {
            assert!(holds_password(endpoint), "{endpoint}");
        }
        for endpoint in [
            "https://example.com/repo.git",
            "ssh://git@example.com:22/repo.git",
            "git@example.com:owner/repo.git",
            "https://example.com/path:with@colon",
            "file:///srv/repo.git",
        ] {
            assert!(!holds_password(endpoint), "{endpoint}");
        }
    }

    #[test]
    fn a_marker_names_one_entry_of_a_folder() {
        for marker in [".git", "Cargo.toml", "x", &"m".repeat(64)] {
            assert!(is_valid_marker(marker), "{marker}");
        }
        for marker in [
            "",
            ".",
            "..",
            "a/b",
            r"a\b",
            "c:",
            "x\u{0}",
            &"m".repeat(65),
        ] {
            assert!(!is_valid_marker(marker), "{marker}");
        }
    }

    #[test]
    fn a_marker_in_the_workspace_or_a_folder_above_it_is_found() {
        let root = tempfile::tempdir().unwrap();
        let repository = root.path().join("repository");
        let project = repository.join("crates").join("app");
        let elsewhere = root.path().join("elsewhere");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(&elsewhere).unwrap();
        // A file counts as well as a folder, as `.git` is a file in a linked
        // worktree.
        fs::write(repository.join(MARKER), b"gitdir: elsewhere").unwrap();

        assert!(holds_marker(&repository, MARKER));
        assert!(holds_marker(&project, MARKER));
        assert!(!holds_marker(&elsewhere, MARKER));
    }

    fn project_at(folder: &Path) -> ProjectView {
        ProjectView {
            id: ProjectId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap(),
            name: "app".to_owned(),
            folder: canonical(folder),
            created_at: UtcMillis::from_millis(0),
        }
    }

    #[test]
    fn each_offering_decides_from_the_conversation_s_workspace() {
        let root = tempfile::tempdir().unwrap();
        let repository = root.path().join("repository");
        let other = root.path().join("other");
        fs::create_dir_all(repository.join("app")).unwrap();
        fs::create_dir_all(&other).unwrap();
        fs::create_dir(repository.join(MARKER)).unwrap();
        let in_repository = project_at(&repository.join("app"));
        let outside = project_at(&other);
        let general = Availability {
            project: None,
            projects: &[],
        };
        let marked = ToolOffering::WhereWorkspaceHolds(MARKER.to_owned());
        // What one run found for each workspace.
        let found = Mutex::new(HashMap::new());
        let fresh = || Mutex::new(HashMap::new());

        assert!(is_offered(&ToolOffering::Always, &fresh(), &general));
        assert!(!is_offered(&ToolOffering::InWorkspace, &fresh(), &general));
        assert!(is_offered(
            &ToolOffering::InWorkspace,
            &fresh(),
            &Availability {
                project: Some(&outside),
                projects: &[],
            }
        ));
        assert!(is_offered(
            &marked,
            &found,
            &Availability {
                project: Some(&in_repository),
                projects: &[],
            }
        ));
        assert!(!is_offered(
            &marked,
            &found,
            &Availability {
                project: Some(&outside),
                projects: &[],
            }
        ));
        assert!(!is_offered(&marked, &found, &general));
        // A run keeps what it found for a workspace; the next run looks again.
        fs::create_dir(other.join(MARKER)).unwrap();
        let in_other = Availability {
            project: Some(&outside),
            projects: &[],
        };
        assert!(!is_offered(&marked, &found, &in_other));
        assert!(is_offered(&marked, &fresh(), &in_other));
    }

    #[test]
    fn card_data_stays_as_an_object_within_its_limit_and_failure_codes_keep_the_rules() {
        assert_eq!(
            card_data("shell", Some(json!({ "files": 2 }))),
            Some(r#"{"files":2}"#.to_owned())
        );
        assert_eq!(card_data("shell", Some(json!([1, 2]))), None);
        assert_eq!(
            card_data(
                "shell",
                Some(json!({ "text": "x".repeat(MAX_TOOL_DATA_BYTES) }))
            ),
            None
        );
        assert_eq!(card_data("shell", None), None);
        assert_eq!(
            kept_code("shell", "merge_conflicts".to_owned()),
            "merge_conflicts"
        );
        assert_eq!(kept_code("shell", "Bad-Code".to_owned()), "failed");
    }

    #[test]
    fn a_refusal_with_a_code_ends_the_call_and_one_without_never_runs() {
        assert_eq!(
            refused(
                "shell",
                "This folder is not in a repository.".to_owned(),
                Some("not_a_repository".to_owned()),
                1000
            ),
            PrepareError::Ended(Done {
                outcome: ToolOutcome::Plugin {
                    plugin: "shell".to_owned(),
                    data: None,
                    failure: Some("not_a_repository".to_owned()),
                },
                output: "This folder is not in a repository.".to_owned(),
                archives: Vec::new(),
            })
        );
        for code in [None, Some("Not Valid".to_owned())] {
            assert_eq!(
                refused("shell", "the path is empty".to_owned(), code, 1000),
                PrepareError::InvalidArguments {
                    detail: "the path is empty".to_owned()
                }
            );
        }
    }

    fn remote(target: &str, label: &str, command: &str) -> ToolAccess {
        ToolAccess::RemoteCommand(RemoteCommandAccess::new(
            target.to_owned(),
            label.to_owned(),
            "deploy@build-box.lan:22".to_owned(),
            "SHA256:x".to_owned(),
            command.to_owned(),
        ))
    }

    /// A remote command that runs in the background.
    fn background(target: &str, label: &str, command: &str) -> ToolAccess {
        ToolAccess::RemoteCommand(
            RemoteCommandAccess::new(
                target.to_owned(),
                label.to_owned(),
                "deploy@build-box.lan:22".to_owned(),
                "SHA256:x".to_owned(),
                command.to_owned(),
            )
            .in_background(),
        )
    }

    fn invalid() -> PrepareError {
        PrepareError::Failed(ToolFailure::PluginInvalid {
            plugin: "shell".to_owned(),
        })
    }

    #[test]
    fn a_tool_with_side_effects_never_reaches_the_explore_agent() {
        let reads = tool("notes_find");
        let reads_files = tool("git_log").with_file_reads();
        let acts = tool("run_on_host").with_side_effects();
        let offered = |entry: &PluginTool| -> Vec<&str> {
            [CONVERSATION, EXPLORE, GENERAL]
                .iter()
                .filter(|agent| offers(entry, agent))
                .map(|agent| agent.name)
                .collect()
        };

        assert_eq!(offered(&reads), ["conversation", "explore", "general"]);
        assert_eq!(
            offered(&reads_files),
            ["conversation", "explore", "general"]
        );
        assert_eq!(offered(&acts), ["conversation", "general"]);
    }

    #[test]
    fn a_remote_command_becomes_the_core_s_access_with_the_plugin_s_id() {
        let accesses = core_accesses(
            "shell",
            ToolEffect::SideEffects,
            vec![remote("key-1", "build-box", "uptime")],
        )
        .unwrap();

        assert_eq!(
            accesses,
            vec![Access::RemoteCommand {
                plugin: "shell".to_owned(),
                target: "key-1".to_owned(),
                label: "build-box".to_owned(),
                endpoint: "deploy@build-box.lan:22".to_owned(),
                identity: "SHA256:x".to_owned(),
                command: "uptime".to_owned(),
                background: false,
            }]
        );
    }

    #[test]
    fn a_remote_command_in_the_background_keeps_its_mode_both_ways() {
        let accesses = core_accesses(
            "shell",
            ToolEffect::SideEffects,
            vec![background("key-1", "build-box", "journalctl -f")],
        )
        .unwrap();

        assert!(matches!(
            &accesses[..],
            [Access::RemoteCommand {
                background: true,
                ..
            }]
        ));
        assert_eq!(
            sdk_accesses(&accesses),
            vec![background("key-1", "build-box", "journalctl -f")]
        );
    }

    #[test]
    fn accesses_that_break_the_sdk_s_rules_are_refused() {
        let cases = [
            (
                ToolEffect::ReadsOwnData,
                vec![remote("key-1", "box", "uptime")],
            ),
            (ToolEffect::SideEffects, Vec::new()),
            (ToolEffect::SideEffects, vec![remote("", "box", "uptime")]),
            (ToolEffect::SideEffects, vec![remote("key-1", "", "uptime")]),
            (
                ToolEffect::SideEffects,
                vec![remote("key-1", "box\u{7}", "uptime")],
            ),
            (ToolEffect::SideEffects, vec![remote("key-1", "box", "   ")]),
            (
                ToolEffect::SideEffects,
                vec![remote("key-1", "box", "up\0time")],
            ),
            (
                ToolEffect::SideEffects,
                vec![remote(&"k".repeat(129), "box", "uptime")],
            ),
            (
                ToolEffect::SideEffects,
                (0..17).map(|_| remote("key-1", "box", "uptime")).collect(),
            ),
            // A call hands the core one command in the background at most.
            (
                ToolEffect::SideEffects,
                vec![
                    background("key-1", "box", "journalctl -f"),
                    background("key-1", "box", "tail -f /var/log/syslog"),
                ],
            ),
        ];
        for (effect, accesses) in cases {
            assert_eq!(
                core_accesses("shell", effect, accesses.clone()),
                Err(invalid()),
                "{effect:?} {accesses:?}"
            );
        }
        // A command may span lines; it then only offers "Allow once".
        let multiline = core_accesses(
            "shell",
            ToolEffect::SideEffects,
            vec![remote("key-1", "box", "cd /srv\nmake")],
        );
        assert_eq!(multiline.map(|accesses| accesses.len()), Ok(1));
        assert_eq!(
            core_accesses("shell", ToolEffect::ReadsOwnData, Vec::new()),
            Ok(Vec::new())
        );
    }

    #[test]
    fn only_archives_under_reserved_ids_with_a_name_and_within_the_limit_are_kept() {
        let id = |number: u32| {
            ArchiveId::parse(&format!("0199e0f2-0000-7000-8000-{number:012}")).unwrap()
        };
        let reserved = [id(1), id(2)];
        let archives = vec![
            ToolArchive::new(id(1), "stdout", "out".to_owned()),
            // The same ID twice, an ID that was not reserved, no name and too
            // much content are left out.
            ToolArchive::new(id(1), "stdout", "again".to_owned()),
            ToolArchive::new(id(3), "stderr", "err".to_owned()),
            ToolArchive::new(id(2), "", "err".to_owned()),
            ToolArchive::new(id(2), "stderr", "x".repeat(MAX_TOOL_ARCHIVE_BYTES + 1)),
        ];

        let kept = kept_archives("shell", &reserved, archives);

        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, id(1));
        assert_eq!(kept[0].content, "out");
    }

    /// A tool with a name and a schema, which the checks read and nothing
    /// runs.
    #[derive(Debug)]
    struct Named {
        name: String,
        parameters: Value,
    }

    impl Tool for Named {
        fn spec(&self) -> ToolSpec {
            ToolSpec::new(self.name.clone(), "Does nothing.", self.parameters.clone())
        }

        fn prepare(
            &self,
            _call: ToolCall,
        ) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
            async { Err(ToolRefusal::new("nothing runs")) }.boxed()
        }

        fn execute(&self, _execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
            async { ToolResult::new("Nothing ran.") }.boxed()
        }
    }

    fn tool(name: &str) -> PluginTool {
        with_schema(name, json!({ "type": "object", "properties": {} }))
    }

    fn with_schema(name: &str, parameters: Value) -> PluginTool {
        PluginTool::new(
            Arc::new(Named {
                name: name.to_owned(),
                parameters,
            }),
            AgentSelector::Every,
        )
    }

    /// A plugin's child agent, which the core does not know.
    const REVIEWER: AgentDefinition = AgentDefinition {
        name: "reviewer",
        ..EXPLORE
    };

    #[test]
    fn each_selector_offers_the_tool_to_its_agents() {
        let agents = [CONVERSATION, EXPLORE, GENERAL, REVIEWER];
        let offered = |selector: AgentSelector| -> Vec<&str> {
            agents
                .iter()
                .filter(|agent| offered_to(&selector, agent))
                .map(|agent| agent.name)
                .collect()
        };

        assert_eq!(offered(AgentSelector::Conversations), ["conversation"]);
        assert_eq!(
            offered(AgentSelector::ChildAgents),
            ["explore", "general", "reviewer"]
        );
        assert_eq!(
            offered(AgentSelector::Every),
            ["conversation", "explore", "general", "reviewer"]
        );
        assert_eq!(
            offered(AgentSelector::Agents(vec![
                "reviewer".to_owned(),
                "explore".to_owned(),
                "missing".to_owned(),
            ])),
            ["explore", "reviewer"]
        );
        assert_eq!(
            offered(AgentSelector::Agents(Vec::new())),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn a_name_is_lower_case_snake_case_of_at_most_64_characters() {
        let longest = format!("n{}", "o".repeat(MAX_TOOL_NAME_CHARS - 1));
        for valid in ["notes", "notes_find", "n2", "a_b_c_", &longest] {
            assert!(is_valid_tool_name(valid), "{valid}");
        }
        let too_long = format!("{longest}o");
        for invalid in [
            "",
            "_notes",
            "2notes",
            "Notes",
            "notes-find",
            "notes.find",
            "notes find",
            "nötes",
            &too_long,
        ] {
            assert!(!is_valid_tool_name(invalid), "{invalid}");
        }
    }

    #[test]
    fn the_check_refuses_core_task_executor_and_taken_names_invalid_names_and_the_activity() {
        let registry = ToolRegistry::with_system_tools();
        // The point and the code of the check's refusal, and whether its
        // detail names the refused tool.
        let check = |tools: &[PluginTool]| {
            check_plugin_tools(&registry, &["echo_text"], &["delegate_task"], tools)
                .err()
                .map(|refusal| {
                    let named = tools
                        .iter()
                        .any(|tool| refusal.detail.contains(tool.spec.name.as_str()));
                    (refusal.point, refusal.code, named)
                })
        };
        let duplicate = |_name: &str| Some(("tools", ID_TAKEN, true));

        assert_eq!(check(&[tool("notes_find"), tool("notes_add")]), None);
        assert_eq!(check(&[tool("read_file")]), duplicate("read_file"));
        assert_eq!(check(&[tool("start_task")]), duplicate("start_task"));
        assert_eq!(
            check(&[tool("wait_for_tasks")]),
            duplicate("wait_for_tasks")
        );
        assert_eq!(check(&[tool("echo_text")]), duplicate("echo_text"));
        assert_eq!(check(&[tool("delegate_task")]), duplicate("delegate_task"));
        assert_eq!(
            check(&[tool("notes_find"), tool("notes_find")]),
            duplicate("notes_find")
        );
        assert_eq!(
            check(&[tool("notes_find"), tool("Notes-Add")]),
            Some(("tools", INVALID_NAME, true))
        );
        assert_eq!(
            check(&[with_schema(
                "notes_label",
                json!({
                    "type": "object",
                    "properties": { "activity": { "type": "string" } },
                }),
            )]),
            Some(("tools", RESERVED_ARGUMENT, true))
        );
        assert_eq!(
            check(&[tool("notes_find")
                .offered_where(ToolOffering::WhereWorkspaceHolds("../outside".to_owned()))]),
            Some(("tools", INVALID_NAME, true))
        );
        assert_eq!(
            check(&[tool("notes_find")
                .offered_where(ToolOffering::WhereWorkspaceHolds(".git".to_owned()))]),
            None
        );
    }

    #[test]
    fn an_output_within_the_limit_stays_and_a_longer_one_is_cut_at_a_character_with_a_note() {
        let limit = 300;
        let short = "é".repeat(limit / 2);
        // An odd start puts every cut of the two-byte characters between
        // their bytes, unless the cut moves to a character.
        let long = format!("x{}", "é".repeat(limit));

        let kept = within_limit(short.clone(), limit);
        let cut = within_limit(long.clone(), limit);

        assert_eq!(kept, short);
        let note = format!("\n{}", texts::plugin_output_cut(long.len()));
        let head = cut
            .strip_suffix(&note)
            .expect("the output ends with the note");
        assert!(long.starts_with(head));
        // The head fills the room before the note, but for the byte of a
        // character that did not fit.
        let room = limit - note.len();
        assert!(
            head.len() == room || head.len() + 1 == room,
            "{}",
            head.len()
        );
        assert!(cut.len() <= limit, "{}", cut.len());
    }
}
