//! Agent definitions (docs/architecture/extension-points.md, agent
//! definitions): what each kind of agent is, whom it answers to, its
//! instructions, its tools and its budgets. A feature that needs a new kind of
//! agent registers a definition; it does not change the loop.
//!
//! Plugins contribute agents through the SDK's `points::agents` (ADR-0034,
//! decision 6), whose consumer is the runtime's registry: this module checks
//! their names, finds them in a snapshot of the registry, and keeps which
//! plugin each one came from.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{PoisonError, RwLock};

use mannie_plugin_sdk::contributions::{Contributions, ExtensionPoint};
use mannie_plugin_sdk::host::{ContributionRefusal, ID_TAKEN};
use mannie_plugin_sdk::ids::PluginId;
use mannie_plugin_sdk::points::agents::{AgentSpec, Agents, Requester as SdkRequester};
use mannie_types::agents::CUSTOM_AGENT_PREFIX;
use mannie_types::ids::ProfileId;

use crate::Runtime;
use crate::plugins::Snapshot;
use crate::questions::ASK_OWNER;
use crate::skills::READ_SKILL;
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

/// A kind of agent. The built-in ones are constants, and the owner's are read
/// from the store (ADR-0059).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDefinition {
    /// Its unique name, as runs record it.
    pub name: Cow<'static, str>,
    /// The built-in agent that plugins see it as, its own name for a built-in
    /// agent, so a plugin's selectors and sections need no change for a custom
    /// agent (ADR-0059, decision 3).
    pub base: &'static str,
    /// Whom it answers to.
    pub requester: Requester,
    /// Whether it is offered only what reads (ADR-0059, decision 5).
    pub read_only: bool,
    /// What other agents read when they choose a child. Empty for none.
    pub description: Cow<'static, str>,
    /// Its own instruction section, after the base instructions. Empty for
    /// none.
    pub instructions: Cow<'static, str>,
    /// The names of the tools it may use: of those the registry holds, and
    /// the runtime's task tools.
    pub tools: Cow<'static, [&'static str]>,
    /// The plugins whose tools and sections it gets, or `None` for every
    /// plugin.
    pub plugins: Option<Vec<String>>,
    /// The child agents it may give tasks to, or `None` for every one.
    pub children: Option<Vec<String>>,
    /// Its own provider profile, or `None` to use the conversation's or the
    /// parent's.
    pub profile: Option<ProfileId>,
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

impl AgentDefinition {
    /// Whether it gets the tools and sections of this plugin (ADR-0059,
    /// decision 4).
    pub(crate) fn gets_plugin(&self, plugin: &str) -> bool {
        self.plugins
            .as_ref()
            .is_none_or(|plugins| plugins.iter().any(|name| name == plugin))
    }
}

/// A plugin's kind of agent, as the runtime runs it (ADR-0034, decision 6).
impl From<&AgentSpec> for AgentDefinition {
    fn from(spec: &AgentSpec) -> Self {
        Self {
            name: Cow::Borrowed(spec.name),
            base: spec.name,
            requester: spec.requester.into(),
            read_only: false,
            description: Cow::Borrowed(""),
            instructions: Cow::Borrowed(spec.instructions),
            tools: Cow::Borrowed(spec.tools),
            plugins: None,
            children: None,
            profile: None,
            max_continuation_notes: spec.budgets.continuation_notes,
            max_tool_rounds: spec.budgets.tool_rounds,
            max_tool_calls: spec.budgets.tool_calls,
            max_tasks: spec.budgets.tasks,
        }
    }
}

/// The system tools (docs/features/system-tools/tools-and-results.md).
/// The commands' tools exist only where the registry holds them, the tools
/// that read and stop background commands come only with `start_command`
/// (ADR-0041), and `move_to_project` is offered only to a general
/// conversation while the owner has projects.
pub const SYSTEM_TOOLS: [&str; 18] = [
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
    "start_command",
    "read_command_output",
    "stop_command",
    "read_tool_output",
    "move_to_project",
];

/// The conversation agent's tools: the system tools, the task tools, the
/// tool that asks the owner questions, which only the agent that answers to
/// the owner has (ADR-0058, decision 9), and the tool that reads skills, which
/// only the desktop app registers (ADR-0061, decision 1).
const CONVERSATION_TOOLS: [&str; 23] = [
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
    SYSTEM_TOOLS[15],
    SYSTEM_TOOLS[16],
    SYSTEM_TOOLS[17],
    TASK_TOOLS[0],
    TASK_TOOLS[1],
    TASK_TOOLS[2],
    ASK_OWNER,
    READ_SKILL,
];

/// The tools of an explore child: the system tools that only read. It has no
/// `run_command`, since a command can change anything.
pub const EXPLORE_TOOLS: [&str; 7] = [
    "system_info",
    "list_directory",
    "file_info",
    "find_files",
    "search_text",
    "read_file",
    "read_tool_output",
];

/// The tools of a general child: every system tool but `move_to_project`,
/// since a child never moves its parent's conversation. Its background
/// commands end with its task (ADR-0041, decision 9).
pub const GENERAL_TOOLS: [&str; 17] = [
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
    "start_command",
    "read_command_output",
    "stop_command",
    "read_tool_output",
];

/// The explore child's tools: those that read, with `read_skill`, which reads
/// only and is offered where the conversation has skills (ADR-0061, decision
/// 10). [`EXPLORE_TOOLS`] stays the group of reads that custom agents choose.
const EXPLORE_AGENT_TOOLS: [&str; 8] = [
    EXPLORE_TOOLS[0],
    EXPLORE_TOOLS[1],
    EXPLORE_TOOLS[2],
    EXPLORE_TOOLS[3],
    EXPLORE_TOOLS[4],
    EXPLORE_TOOLS[5],
    EXPLORE_TOOLS[6],
    READ_SKILL,
];

/// The general child's tools: those of [`GENERAL_TOOLS`], with `read_skill`.
const GENERAL_AGENT_TOOLS: [&str; 18] = [
    GENERAL_TOOLS[0],
    GENERAL_TOOLS[1],
    GENERAL_TOOLS[2],
    GENERAL_TOOLS[3],
    GENERAL_TOOLS[4],
    GENERAL_TOOLS[5],
    GENERAL_TOOLS[6],
    GENERAL_TOOLS[7],
    GENERAL_TOOLS[8],
    GENERAL_TOOLS[9],
    GENERAL_TOOLS[10],
    GENERAL_TOOLS[11],
    GENERAL_TOOLS[12],
    GENERAL_TOOLS[13],
    GENERAL_TOOLS[14],
    GENERAL_TOOLS[15],
    GENERAL_TOOLS[16],
    READ_SKILL,
];

/// The conversation agent: the base instructions, the system tools and the
/// task tools, with 100 rounds and 250 calls of tools in one reply (owner
/// decision of 2026-10-14) and 20 tasks.
pub const CONVERSATION: AgentDefinition = AgentDefinition {
    name: Cow::Borrowed(CONVERSATION_AGENT),
    base: CONVERSATION_AGENT,
    requester: Requester::Owner,
    read_only: false,
    description: Cow::Borrowed(""),
    instructions: Cow::Borrowed(""),
    tools: Cow::Borrowed(&CONVERSATION_TOOLS),
    plugins: None,
    children: None,
    profile: None,
    max_continuation_notes: 2,
    max_tool_rounds: 100,
    max_tool_calls: 250,
    max_tasks: 20,
};

/// The explore child agent: the read-only tools, with 50 rounds and 150 calls
/// of tools for one task.
pub const EXPLORE: AgentDefinition = AgentDefinition {
    name: Cow::Borrowed(EXPLORE_AGENT),
    base: EXPLORE_AGENT,
    requester: Requester::Agent,
    read_only: true,
    description: Cow::Borrowed(texts::EXPLORE_DESCRIPTION),
    instructions: Cow::Borrowed(texts::EXPLORE_INSTRUCTIONS),
    tools: Cow::Borrowed(&EXPLORE_AGENT_TOOLS),
    plugins: None,
    children: None,
    profile: None,
    max_continuation_notes: 2,
    max_tool_rounds: 50,
    max_tool_calls: 150,
    max_tasks: 0,
};

/// The general child agent: every system tool but the move into a project,
/// with the conversation agent's budgets of tools.
pub const GENERAL: AgentDefinition = AgentDefinition {
    name: Cow::Borrowed(GENERAL_AGENT),
    base: GENERAL_AGENT,
    requester: Requester::Agent,
    read_only: false,
    description: Cow::Borrowed(texts::GENERAL_DESCRIPTION),
    instructions: Cow::Borrowed(texts::GENERAL_INSTRUCTIONS),
    tools: Cow::Borrowed(&GENERAL_AGENT_TOOLS),
    plugins: None,
    children: None,
    profile: None,
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
    pub name: String,
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
        if self.get(&definition.name).is_some() {
            return Err(DuplicateAgent {
                name: definition.name.into_owned(),
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

    /// Every definition, in the order of registration.
    pub fn definitions(&self) -> impl Iterator<Item = &AgentDefinition> {
        self.definitions.iter()
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
    /// (the rule `Global`), and those with the prefix of the owner's agents, so
    /// a plugin that is turned on later cannot take over a stored agent, whose
    /// tools may have been chosen to be narrow (T-85).
    pub(crate) fn check_agents(
        &self,
        snapshot: &Snapshot,
        contributions: &Contributions,
    ) -> Result<(), ContributionRefusal> {
        for agent in contributions.entries::<Agents>() {
            let taken = agent.name.starts_with(CUSTOM_AGENT_PREFIX)
                || self.inner.agents.get(agent.name).is_some()
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

    /// The definition of an agent: the core's, that of a plugin in the
    /// snapshot, or the owner's (ADR-0059, decision 1).
    pub(crate) fn agent_in(&self, snapshot: &Snapshot, name: &str) -> Option<AgentDefinition> {
        if let Some(definition) = self.inner.agents.get(name) {
            return Some(definition.clone());
        }
        if let Some(registered) = snapshot.find::<Agents>(name) {
            return Some(AgentDefinition::from(&registered.entry));
        }
        self.inner.custom_agents.get(name)
    }

    /// The agent that answers to the owner with this key, or `None`.
    pub(crate) fn conversation_agent(&self, key: &str) -> Option<AgentDefinition> {
        self.agent(key)
            .filter(|definition| definition.requester == Requester::Owner)
    }

    /// The name of the agent of a conversation's replies: the one that it
    /// names, or the built-in one for none and for a key that no conversation
    /// agent has, such as a deleted agent's (ADR-0059, decision 6).
    pub(crate) fn conversation_agent_name(&self, agent_key: Option<&str>) -> String {
        agent_key
            .filter(|key| self.conversation_agent(key).is_some())
            .unwrap_or(CONVERSATION_AGENT)
            .to_owned()
    }

    /// Whether an agent of this name can start and read background commands. A
    /// conversation's commands outlive a switch of agents, so one that cannot
    /// reads no word of a tool that it lacks, and no end wakes it (ADR-0059,
    /// decision 13).
    pub(crate) fn agent_reads_commands(&self, agent: &str) -> bool {
        self.agent(agent)
            .is_some_and(|definition| definition.tools.contains(&"start_command"))
    }

    /// The child agents that a run of this agent may give tasks to: the ones it
    /// names, and for an agent that only reads, only those that only read.
    pub(crate) fn roster_of(&self, snapshot: &Snapshot, agent: &AgentDefinition) -> Vec<String> {
        self.child_agent_names(snapshot)
            .into_iter()
            .filter(|name| {
                agent
                    .children
                    .as_ref()
                    .is_none_or(|children| children.contains(name))
            })
            .filter(|name| {
                !agent.read_only
                    || self
                        .agent_in(snapshot, name)
                        .is_some_and(|definition| definition.read_only)
            })
            .collect()
    }

    /// The kinds of child agents that `start_task` can start: the core's,
    /// those of the plugins in the snapshot and the owner's.
    pub(crate) fn child_agent_names(&self, snapshot: &Snapshot) -> Vec<String> {
        let plugin_children = snapshot
            .items::<Agents>()
            .iter()
            .filter(|registered| registered.entry.requester == SdkRequester::Agent)
            .map(|registered| registered.entry.name.to_owned());
        self.inner
            .agents
            .child_agents()
            .map(|definition| definition.name.clone().into_owned())
            .chain(plugin_children)
            .chain(self.inner.custom_agents.child_keys())
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
    use std::borrow::Cow;

    use mannie_plugin_sdk::points::agents::{AgentBudgets, AgentSpec, Requester as SdkRequester};
    use mannie_tools::{PermissionClass, ToolRegistry};

    use super::{
        AgentDefinition, AgentRegistry, CONVERSATION, CONVERSATION_AGENT, DuplicateAgent, EXPLORE,
        GENERAL, Requester,
    };
    use crate::questions::ASK_OWNER;
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
            name: Cow::Borrowed("fake"),
            base: "fake",
            requester: Requester::Agent,
            read_only: false,
            description: Cow::Borrowed(""),
            instructions: Cow::Borrowed("Do fake things."),
            tools: Cow::Borrowed(&[]),
            plugins: None,
            children: None,
            profile: None,
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
                .map(|definition| definition.instructions.as_ref()),
            Some("Do fake things.")
        );
        assert!(registry.get(CONVERSATION_AGENT).is_some());
        assert_eq!(
            duplicate,
            Err(DuplicateAgent {
                name: "fake".to_owned()
            })
        );
        let children: Vec<_> = registry
            .child_agents()
            .map(|definition| definition.name.as_ref())
            .collect();
        assert_eq!(children, ["explore", "general", "fake"]);
    }

    #[test]
    fn a_plugin_s_agent_spec_becomes_a_definition_with_each_field_in_its_place() {
        let definition = AgentDefinition::from(&HELPER);

        assert_eq!(
            definition,
            AgentDefinition {
                name: Cow::Borrowed("helper"),
                base: "helper",
                requester: Requester::Agent,
                read_only: false,
                description: Cow::Borrowed(""),
                instructions: Cow::Borrowed("Help with it."),
                tools: Cow::Borrowed(&["read_file"]),
                plugins: None,
                children: None,
                profile: None,
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
        let mut registry = ToolRegistry::with_system_tools();
        registry.register(mannie_tools::read_skill_tool()).unwrap();

        for name in EXPLORE.tools.iter() {
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

    #[test]
    fn only_the_conversation_agent_asks_the_owner_questions() {
        assert!(CONVERSATION.tools.contains(&ASK_OWNER));
        for child in [EXPLORE, GENERAL] {
            assert!(!child.tools.contains(&ASK_OWNER), "{}", child.name);
        }
    }
}
