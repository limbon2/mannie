//! Agent definitions (docs/architecture/extension-points.md, agent
//! definitions): what each kind of agent is, whom it answers to, its
//! instructions, its tools and its budgets. A feature that needs a new kind of
//! agent registers a definition; it does not change the loop.
//!
//! Plugins contribute agents through the SDK's `points::agents` (ADR-0034,
//! decision 6), whose consumer is the runtime's registry: this module checks
//! their names, finds them in a snapshot of the registry, and keeps which
//! plugin each one came from.

use std::collections::HashMap;
use std::sync::{PoisonError, RwLock};

use mannie_plugin_sdk::contributions::{Contributions, ExtensionPoint};
use mannie_plugin_sdk::host::{ContributionRefusal, ID_TAKEN};
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::points::agents::{AgentSpec, Agents, Requester as SdkRequester};

use crate::Runtime;
use crate::plugins::Snapshot;
use crate::tasks::TASK_TOOLS;
use crate::texts;

/// The name of the agent of the owner's conversations.
pub const CONVERSATION_AGENT: &str = "conversation";

/// The name of the child agent that reads and searches.
pub const EXPLORE_AGENT: &str = "explore";

/// The name of the child agent that also changes files and runs commands.
pub const GENERAL_AGENT: &str = "general";

/// Whom an agent answers to, which decides how the text it reads addresses
/// it (docs/features/child-agents/tasks.md, the text the model sees).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requester {
    /// The owner: the agent of one of the owner's conversations.
    Owner,
    /// Another agent, which gave it a task: a child agent.
    Agent,
}

impl From<SdkRequester> for Requester {
    fn from(requester: SdkRequester) -> Self {
        match requester {
            SdkRequester::Owner => Self::Owner,
            SdkRequester::Agent => Self::Agent,
        }
    }
}

impl From<Requester> for SdkRequester {
    fn from(requester: Requester) -> Self {
        match requester {
            Requester::Owner => Self::Owner,
            Requester::Agent => Self::Agent,
        }
    }
}

/// A kind of agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDefinition {
    /// Its unique name, as runs record it.
    pub name: &'static str,
    /// Whom it answers to.
    pub requester: Requester,
    /// Its own instruction section, after the base instructions. Empty for
    /// none.
    pub instructions: &'static str,
    /// The names of the tools it may use: of those the registry holds, and
    /// the runtime's task tools.
    pub tools: &'static [&'static str],
    /// The most continuation notes in one turn (docs/architecture/
    /// agent-runtime.md, model quirks and early stops).
    pub max_continuation_notes: u32,
    /// The most model calls with tool calls in one reply.
    pub max_tool_rounds: u32,
    /// The most tool calls in one reply.
    pub max_tool_calls: u32,
    /// The most tasks that one reply may start or follow up.
    pub max_tasks: u32,
}

/// A plugin's kind of agent, as the runtime runs it (ADR-0034, decision 6).
impl From<&AgentSpec> for AgentDefinition {
    fn from(spec: &AgentSpec) -> Self {
        Self {
            name: spec.name,
            requester: spec.requester.into(),
            instructions: spec.instructions,
            tools: spec.tools,
            max_continuation_notes: spec.budgets.continuation_notes,
            max_tool_rounds: spec.budgets.tool_rounds,
            max_tool_calls: spec.budgets.tool_calls,
            max_tasks: spec.budgets.tasks,
        }
    }
}

/// The system tools (docs/features/system-tools/tools-and-results.md).
/// `run_command` exists only where the registry holds it, and
/// `move_to_project` is offered only to a general conversation while the
/// owner has projects.
pub const SYSTEM_TOOLS: [&str; 15] = [
    "system_info",
    "list_directory",
    "file_info",
    "find_files",
    "search_text",
    "read_file",
    "write_file",
    "edit_file",
    "create_directory",
    "copy_path",
    "move_path",
    "delete_path",
    "run_command",
    "read_tool_output",
    "move_to_project",
];

/// The conversation agent's tools: the system tools and the task tools.
const CONVERSATION_TOOLS: [&str; 18] = [
    SYSTEM_TOOLS[0],
    SYSTEM_TOOLS[1],
    SYSTEM_TOOLS[2],
    SYSTEM_TOOLS[3],
    SYSTEM_TOOLS[4],
    SYSTEM_TOOLS[5],
    SYSTEM_TOOLS[6],
    SYSTEM_TOOLS[7],
    SYSTEM_TOOLS[8],
    SYSTEM_TOOLS[9],
    SYSTEM_TOOLS[10],
    SYSTEM_TOOLS[11],
    SYSTEM_TOOLS[12],
    SYSTEM_TOOLS[13],
    SYSTEM_TOOLS[14],
    TASK_TOOLS[0],
    TASK_TOOLS[1],
    TASK_TOOLS[2],
];

/// The tools of an explore child: the system tools that only read. It has no
/// `run_command`, since a command can change anything.
const EXPLORE_TOOLS: [&str; 7] = [
    "system_info",
    "list_directory",
    "file_info",
    "find_files",
    "search_text",
    "read_file",
    "read_tool_output",
];

/// The tools of a general child: every system tool but `move_to_project`,
/// since a child never moves its parent's conversation.
const GENERAL_TOOLS: [&str; 14] = [
    "system_info",
    "list_directory",
    "file_info",
    "find_files",
    "search_text",
    "read_file",
    "write_file",
    "edit_file",
    "create_directory",
    "copy_path",
    "move_path",
    "delete_path",
    "run_command",
    "read_tool_output",
];

/// The conversation agent: the base instructions, the system tools and the
/// task tools, with 100 rounds and 250 calls of tools in one reply (owner
/// decision of 2026-10-14) and 20 tasks.
pub const CONVERSATION: AgentDefinition = AgentDefinition {
    name: CONVERSATION_AGENT,
    requester: Requester::Owner,
    instructions: "",
    tools: &CONVERSATION_TOOLS,
    max_continuation_notes: 2,
    max_tool_rounds: 100,
    max_tool_calls: 250,
    max_tasks: 20,
};

/// The explore child agent: the read-only tools, with 50 rounds and 150 calls
/// of tools for one task.
pub const EXPLORE: AgentDefinition = AgentDefinition {
    name: EXPLORE_AGENT,
    requester: Requester::Agent,
    instructions: texts::EXPLORE_INSTRUCTIONS,
    tools: &EXPLORE_TOOLS,
    max_continuation_notes: 2,
    max_tool_rounds: 50,
    max_tool_calls: 150,
    max_tasks: 0,
};

/// The general child agent: every system tool but the move into a project,
/// with the conversation agent's budgets of tools.
pub const GENERAL: AgentDefinition = AgentDefinition {
    name: GENERAL_AGENT,
    requester: Requester::Agent,
    instructions: texts::GENERAL_INSTRUCTIONS,
    tools: &GENERAL_TOOLS,
    max_continuation_notes: 2,
    max_tool_rounds: 100,
    max_tool_calls: 250,
    max_tasks: 0,
};

/// A definition whose name is already registered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the agent {name} is already registered")]
pub struct DuplicateAgent {
    /// The name.
    pub name: &'static str,
}

/// The registered agent definitions.
#[derive(Debug, Clone, Default)]
pub struct AgentRegistry {
    definitions: Vec<AgentDefinition>,
}

impl AgentRegistry {
    /// A registry without definitions.
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry with Mannie's definitions: the conversation agent and the
    /// explore and general child agents.
    pub fn with_builtin() -> Self {
        Self {
            definitions: vec![CONVERSATION, EXPLORE, GENERAL],
        }
    }

    /// Registers a definition. Names are unique.
    pub fn register(&mut self, definition: AgentDefinition) -> Result<(), DuplicateAgent> {
        if self.get(definition.name).is_some() {
            return Err(DuplicateAgent {
                name: definition.name,
            });
        }
        self.definitions.push(definition);
        Ok(())
    }

    /// The definition with this name.
    pub fn get(&self, name: &str) -> Option<&AgentDefinition> {
        self.definitions
            .iter()
            .find(|definition| definition.name == name)
    }

    /// The definitions that can be given tasks: those of agents that answer
    /// to another agent, in the order of registration.
    pub fn child_agents(&self) -> impl Iterator<Item = &AgentDefinition> {
        self.definitions
            .iter()
            .filter(|definition| definition.requester == Requester::Agent)
    }
}

/// The plugin of each agent that a plugin registered since Mannie started,
/// kept after the plugin stops, so a call that names the agent of a plugin
/// that is off says which plugin it was.
#[derive(Debug, Default)]
pub(crate) struct AgentPlugins {
    plugins: RwLock<HashMap<&'static str, String>>,
}

impl AgentPlugins {
    /// Notes the plugin of each of its agents.
    pub(crate) fn record(&self, plugin: &PluginId, agents: &[AgentSpec]) {
        let mut plugins = self.plugins.write().unwrap_or_else(PoisonError::into_inner);
        for agent in agents {
            plugins.insert(agent.name, plugin.as_str().to_owned());
        }
    }

    /// The plugin that registered an agent, also while it is off.
    fn plugin_of(&self, agent: &str) -> Option<String> {
        self.plugins
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(agent)
            .cloned()
    }
}

impl Runtime {
    /// Refuses a plugin's agents whose names the core or another plugin has
    /// (the rule `Global`).
    pub(crate) fn check_agents(
        &self,
        snapshot: &Snapshot,
        contributions: &Contributions,
    ) -> Result<(), ContributionRefusal> {
        for agent in contributions.entries::<Agents>() {
            let taken = self.inner.agents.get(agent.name).is_some()
                || snapshot.find::<Agents>(agent.name).is_some();
            if taken {
                return Err(ContributionRefusal::new(
                    Agents::NAME,
                    ID_TAKEN,
                    format!("an agent named {} is registered already", agent.name),
                ));
            }
        }
        Ok(())
    }

    /// The definition of an agent: the core's or that of a plugin that is on.
    pub(crate) fn agent(&self, name: &str) -> Option<AgentDefinition> {
        self.agent_in(&self.contributions(), name)
    }

    /// The definition of an agent: the core's or that of a plugin in the
    /// snapshot.
    pub(crate) fn agent_in(&self, snapshot: &Snapshot, name: &str) -> Option<AgentDefinition> {
        if let Some(definition) = self.inner.agents.get(name) {
            return Some(definition.clone());
        }
        snapshot
            .find::<Agents>(name)
            .map(|registered| AgentDefinition::from(&registered.entry))
    }

    /// The kinds of child agents that `start_task` can start: the core's
    /// and those of the plugins in the snapshot.
    pub(crate) fn child_agent_names(&self, snapshot: &Snapshot) -> Vec<&'static str> {
        let plugin_children = snapshot
            .items::<Agents>()
            .iter()
            .filter(|registered| registered.entry.requester == SdkRequester::Agent)
            .map(|registered| registered.entry.name);
        self.inner
            .agents
            .child_agents()
            .map(|definition| definition.name)
            .chain(plugin_children)
            .collect()
    }

    /// The plugin that registered an agent since Mannie started, also while
    /// it is off; `None` for the core's agents and unknown names.
    pub(crate) fn agent_plugin(&self, agent: &str) -> Option<String> {
        self.inner.agent_plugins.plugin_of(agent)
    }
}

#[cfg(test)]
mod tests {
    use mannie_plugin_sdk::points::agents::{AgentBudgets, AgentSpec, Requester as SdkRequester};
    use mannie_tools::{PermissionClass, ToolRegistry};

    use super::{
        AgentDefinition, AgentRegistry, CONVERSATION, CONVERSATION_AGENT, DuplicateAgent, EXPLORE,
        GENERAL, Requester,
    };
    use crate::tasks::TASK_TOOLS;

    /// A plugin's agent whose budgets all differ, so a swapped budget shows.
    const HELPER: AgentSpec = AgentSpec::new(
        "helper",
        SdkRequester::Agent,
        "Help with it.",
        &["read_file"],
        AgentBudgets::new(1, 2, 3, 4),
    );

    #[test]
    fn a_registered_definition_is_found_and_names_are_unique() {
        let mut registry = AgentRegistry::with_builtin();
        let fake = AgentDefinition {
            name: "fake",
            requester: Requester::Agent,
            instructions: "Do fake things.",
            tools: &[],
            max_continuation_notes: 0,
            max_tool_rounds: 0,
            max_tool_calls: 0,
            max_tasks: 0,
        };

        registry.register(fake.clone()).unwrap();
        let duplicate = registry.register(fake);

        assert_eq!(
            registry
                .get("fake")
                .map(|definition| definition.instructions),
            Some("Do fake things.")
        );
        assert!(registry.get(CONVERSATION_AGENT).is_some());
        assert_eq!(duplicate, Err(DuplicateAgent { name: "fake" }));
        let children: Vec<_> = registry
            .child_agents()
            .map(|definition| definition.name)
            .collect();
        assert_eq!(children, ["explore", "general", "fake"]);
    }

    #[test]
    fn a_plugin_s_agent_spec_becomes_a_definition_with_each_field_in_its_place() {
        let definition = AgentDefinition::from(&HELPER);

        assert_eq!(
            definition,
            AgentDefinition {
                name: "helper",
                requester: Requester::Agent,
                instructions: "Help with it.",
                tools: &["read_file"],
                max_continuation_notes: 1,
                max_tool_rounds: 2,
                max_tool_calls: 3,
                max_tasks: 4,
            }
        );
        for (sdk, core) in [
            (SdkRequester::Owner, Requester::Owner),
            (SdkRequester::Agent, Requester::Agent),
        ] {
            assert_eq!(Requester::from(sdk), core);
            assert_eq!(SdkRequester::from(core), sdk);
        }
    }

    #[test]
    fn an_explore_child_has_only_tools_that_read() {
        let registry = ToolRegistry::with_system_tools();

        for name in EXPLORE.tools {
            let tool = registry
                .get(name)
                .expect("an explore tool is a system tool");
            assert_eq!(tool.class(), PermissionClass::ReadOnly, "{name}");
        }
    }

    #[test]
    fn children_neither_move_their_parent_nor_give_tasks() {
        for child in [EXPLORE, GENERAL] {
            assert_eq!(child.requester, Requester::Agent);
            assert_eq!(child.max_tasks, 0);
            assert!(!child.tools.contains(&"move_to_project"), "{}", child.name);
            for task_tool in TASK_TOOLS {
                assert!(!child.tools.contains(&task_tool), "{}", child.name);
            }
        }
        for task_tool in TASK_TOOLS {
            assert!(CONVERSATION.tools.contains(&task_tool));
        }
        assert_eq!(CONVERSATION.requester, Requester::Owner);
    }
}
