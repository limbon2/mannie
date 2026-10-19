//! The plugins' tools in the core's tool loop (ADR-0034, decision 6): the
//! check of their names and schemas when their plugin starts, the agents
//! that a tool's selector offers it to, and the adapter through which the
//! core runs the SDK's tool as one of its own. The adapter goes through the
//! same lifecycle as the core's tools (ADR-0013): the call is prepared, the
//! policy decides on its accesses, the call is journaled, and only then does
//! it execute, until it ends or the owner stops the reply.

use std::sync::Arc;

use futures_util::FutureExt;
use futures_util::future::BoxFuture;
use mannie_plugin_sdk::contributions::{Contributions, ExtensionPoint};
use mannie_plugin_sdk::host::{ContributionRefusal, ID_TAKEN, INVALID_NAME, RESERVED_ARGUMENT};
use mannie_plugin_sdk::points::executors::Executors;
use mannie_plugin_sdk::points::tools::{
    AgentSelector, PluginTool, Tool as SdkTool, ToolCall, ToolExecution, ToolSpec, Tools,
};
use mannie_tools::{
    Done, Execution, PermissionClass, PrepareError, Prepared, Tool, ToolContext, ToolRegistry,
    stopped, takes_activity,
};
use mannie_types::tools::ToolOutcome;

use crate::Runtime;
use crate::agents::{AgentDefinition, Requester};
use crate::plugins::Snapshot;
use crate::tasks::TASK_TOOLS;
use crate::texts;

/// The most characters of a tool's name. The providers' interfaces accept
/// names of lower-case letters, digits and underscores up to this length.
const MAX_TOOL_NAME_CHARS: usize = 64;

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
        .filter(|registered| offered_to(&registered.entry.offered_to, agent))
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
            });
            (offered, adapter)
        })
        .collect()
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
    /// The plugin's ID, which the call's outcome names.
    plugin: String,
    /// The plugin's tool.
    tool: Arc<dyn SdkTool>,
    /// What the model is told of the tool, as the plugin's entry read it.
    spec: ToolSpec,
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

    /// SDK 1's tools read only their plugin's own data (ADR-0034, decision
    /// 6), so a call is never an operation with side effects.
    fn class(&self) -> PermissionClass {
        PermissionClass::ReadOnly
    }

    fn prepare(
        &self,
        context: ToolContext,
        arguments: String,
    ) -> BoxFuture<'static, Result<Prepared, PrepareError>> {
        let preparing = self
            .tool
            .prepare(ToolCall::new(arguments, context.conversation_id));
        async move {
            match preparing.await {
                // SDK 1 has no access, so the policy decides on none and
                // the owner is never asked about the call.
                Ok(accesses) => Ok(Prepared {
                    accesses: accesses.into_iter().map(|access| match access {}).collect(),
                }),
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
            stop,
            ..
        } = execution;
        let limit = context.result_limit;
        let running = self
            .tool
            .execute(ToolExecution::new(arguments, context.conversation_id));
        let plugin = self.plugin.clone();
        async move {
            tokio::select! {
                result = running => Done {
                    outcome: ToolOutcome::Plugin { plugin },
                    output: within_limit(result.output, limit),
                    archives: Vec::new(),
                },
                // The SDK cancels a call by dropping its future (ADR-0034,
                // decision 3), so Stop and Shutdown end the plugin's work
                // here, and the run waits for nothing that may never end.
                () = stop.stopped() => Done::failed(stopped()),
            }
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures_util::FutureExt;
    use mannie_plugin_sdk::BoxFuture;
    use mannie_plugin_sdk::points::tools::{
        AgentSelector, PluginTool, Tool, ToolAccess, ToolCall, ToolExecution, ToolRefusal,
        ToolResult, ToolSpec,
    };
    use mannie_tools::ToolRegistry;
    use serde_json::{Value, json};

    use mannie_plugin_sdk::host::{ID_TAKEN, INVALID_NAME, RESERVED_ARGUMENT};

    use super::{
        MAX_TOOL_NAME_CHARS, check_plugin_tools, is_valid_tool_name, offered_to, within_limit,
    };
    use crate::agents::{AgentDefinition, CONVERSATION, EXPLORE, GENERAL};
    use crate::texts;

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
