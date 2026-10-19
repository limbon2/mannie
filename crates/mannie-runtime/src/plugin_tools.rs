//! The plugins' tools in the core's tool loop (ADR-0034, decision 6): the
//! check of their names and schemas when their plugin starts, the agents
//! that a tool's selector offers it to, and the adapter through which the
//! core runs the SDK's tool as one of its own. The adapter goes through the
//! same lifecycle as the core's tools (ADR-0013): the call is prepared, the
//! policy decides on its accesses, the call is journaled, and only then does
//! it execute, until it ends or the owner stops the reply.
//!
//! A tool with side effects returns the accesses on which the policy
//! decides, such as a command on a remote host (ADR-0036). The adapter turns
//! them into the core's accesses with the plugin's ID, so a plugin never
//! speaks for another, and refuses what breaks the SDK's rules before
//! anything runs.

use std::collections::HashSet;
use std::sync::Arc;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_plugin_sdk::contributions::{Contributions, ExtensionPoint};
use mannie_plugin_sdk::host::{ContributionRefusal, ID_TAKEN, INVALID_NAME, RESERVED_ARGUMENT};
use mannie_plugin_sdk::points::executors::Executors;
use mannie_plugin_sdk::points::tools::{
    AgentSelector, MAX_TOOL_ARCHIVE_BYTES, MAX_TOOL_ARCHIVES, PluginTool, RemoteCommandAccess,
    Tool as SdkTool, ToolAccess, ToolArchive, ToolCall, ToolEffect, ToolExecution, ToolSpec, Tools,
};
use mannie_tools::{
    ArchivedOutput, Done, Execution, PermissionClass, PrepareError, Prepared, Tool, ToolContext,
    ToolRegistry, stopped, takes_activity,
};
use mannie_types::ids::ArchiveId;
use mannie_types::tools::{Access, MAX_ARGUMENTS_BYTES, ToolFailure, ToolOutcome};

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
/// runs.
pub(crate) fn offered_tools(
    snapshot: &Snapshot,
    agent: &AgentDefinition,
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
        names.push(name);
    }
    Ok(())
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

    /// A tool that reads only its plugin's own data never performs an
    /// operation (ADR-0034, decision 6); a tool with side effects performs
    /// one in every call, which the journal records (ADR-0036).
    fn class(&self) -> PermissionClass {
        match self.effect {
            ToolEffect::ReadsOwnData => PermissionClass::ReadOnly,
            ToolEffect::SideEffects => PermissionClass::SideEffects,
        }
    }

    fn prepare(
        &self,
        context: ToolContext,
        arguments: String,
    ) -> BoxFuture<'static, Result<Prepared, PrepareError>> {
        let preparing = self
            .tool
            .prepare(ToolCall::new(arguments, context.conversation_id));
        let plugin = self.plugin.clone();
        let effect = self.effect;
        async move {
            match preparing.await {
                Ok(accesses) => {
                    core_accesses(&plugin, effect, accesses).map(|accesses| Prepared { accesses })
                }
                Err(refusal) => Err(PrepareError::InvalidArguments {
                    detail: refusal.detail,
                }),
            }
        }
        .boxed()
    }

    fn execute(&self, execution: Execution) -> BoxFuture<'static, Done> {
        let Execution {
            context,
            arguments,
            accesses,
            stop,
            ..
        } = execution;
        let limit = context.result_limit;
        let reserved: Vec<ArchiveId> = (0..MAX_TOOL_ARCHIVES)
            .map(|_| (context.archive_ids)())
            .collect();
        let running = self.tool.execute(
            ToolExecution::new(arguments, context.conversation_id)
                .with_authorized(sdk_accesses(&accesses))
                .with_result_limit(u32::try_from(limit).unwrap_or(u32::MAX))
                .with_archive_ids(reserved.clone()),
        );
        let plugin = self.plugin.clone();
        async move {
            tokio::select! {
                // A result that is ready wins over a stop that arrives at the
                // same moment, so an operation that finished keeps its result.
                biased;
                result = running => {
                    let archives = kept_archives(&plugin, &reserved, result.archives);
                    Done {
                        outcome: ToolOutcome::Plugin { plugin },
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

/// The core's accesses of a call of a plugin's tool, or the failure of a
/// plugin that broke the SDK's rules: a tool that reads its own data returns
/// no access, a tool with side effects returns at least one, and every access
/// keeps its limits (ADR-0036). Nothing runs after such a failure.
fn core_accesses(
    plugin: &str,
    effect: ToolEffect,
    accesses: Vec<ToolAccess>,
) -> Result<Vec<Access>, PrepareError> {
    let broken = || {
        tracing::error!(
            plugin,
            "a plugin's tool returned accesses that break the SDK's rules"
        );
        PrepareError::Failed(ToolFailure::PluginInvalid {
            plugin: plugin.to_owned(),
        })
    };
    let fits = match effect {
        ToolEffect::ReadsOwnData => accesses.is_empty(),
        ToolEffect::SideEffects => !accesses.is_empty() && accesses.len() <= MAX_ACCESSES,
    };
    if !fits {
        return Err(broken());
    }
    accesses
        .into_iter()
        .map(|access| match access {
            ToolAccess::RemoteCommand(remote) => remote_access(plugin, remote).ok_or_else(broken),
        })
        .collect()
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
                ..
            } => Some(ToolAccess::RemoteCommand(RemoteCommandAccess::new(
                target.clone(),
                label.clone(),
                endpoint.clone(),
                identity.clone(),
                command.clone(),
            ))),
            Access::Path { .. } | Access::Command { .. } | Access::Project { .. } => None,
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
    use std::sync::Arc;

    use futures_util::FutureExt;
    use mannie_plugin_sdk::BoxFuture;
    use mannie_plugin_sdk::points::tools::{
        AgentSelector, MAX_TOOL_ARCHIVE_BYTES, PluginTool, RemoteCommandAccess, Tool, ToolAccess,
        ToolArchive, ToolCall, ToolEffect, ToolExecution, ToolRefusal, ToolResult, ToolSpec,
    };
    use mannie_tools::{PrepareError, ToolRegistry};
    use mannie_types::ids::ArchiveId;
    use mannie_types::tools::{Access, ToolFailure};
    use serde_json::{Value, json};

    use mannie_plugin_sdk::host::{ID_TAKEN, INVALID_NAME, RESERVED_ARGUMENT};

    use super::{
        MAX_TOOL_NAME_CHARS, check_plugin_tools, core_accesses, is_valid_tool_name, kept_archives,
        offered_to, offers, within_limit,
    };
    use crate::agents::{AgentDefinition, CONVERSATION, EXPLORE, GENERAL};
    use crate::texts;

    fn remote(target: &str, label: &str, command: &str) -> ToolAccess {
        ToolAccess::RemoteCommand(RemoteCommandAccess::new(
            target.to_owned(),
            label.to_owned(),
            "deploy@build-box.lan:22".to_owned(),
            "SHA256:x".to_owned(),
            command.to_owned(),
        ))
    }

    fn invalid() -> PrepareError {
        PrepareError::Failed(ToolFailure::PluginInvalid {
            plugin: "shell".to_owned(),
        })
    }

    #[test]
    fn a_tool_with_side_effects_never_reaches_the_explore_agent() {
        let reads = tool("notes_find");
        let acts = tool("run_on_host").with_side_effects();
        let offered = |entry: &PluginTool| -> Vec<&str> {
            [CONVERSATION, EXPLORE, GENERAL]
                .iter()
                .filter(|agent| offers(entry, agent))
                .map(|agent| agent.name)
                .collect()
        };

        assert_eq!(offered(&reads), ["conversation", "explore", "general"]);
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
            }]
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
