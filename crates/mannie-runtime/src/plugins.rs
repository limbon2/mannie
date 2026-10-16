//! What plugins contribute to the runtime while they are on (ADR-0030,
//! decision 5): tools, agent definitions, instruction sections and task
//! executors, each kept with the ID of the plugin that made it.
//!
//! The runtime takes a plugin's contributions when the plugin starts and
//! drops them when it stops. A run reads them once, when it starts, so the
//! tools and the prompt that the model sees never change within a reply
//! (decision 8); a call into a plugin that stopped since does not run, and
//! its result says that the plugin is off.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

use mannie_tools::Tool;

use crate::Runtime;
use crate::agents::AgentDefinition;
use crate::executors::TaskExecutor;
use crate::instructions::InstructionSection;

/// A tool of a plugin, with the agents that are offered it.
#[derive(Debug, Clone)]
pub struct PluginTool {
    /// The tool.
    pub tool: Arc<dyn Tool>,
    /// The names of the agents whose runs are offered the tool, when their
    /// profiles use tools.
    pub agents: Vec<&'static str>,
}

/// What a plugin contributes while it is on.
#[derive(Debug, Default)]
pub struct PluginContributions {
    /// Tools, each with the agents that are offered it.
    pub tools: Vec<PluginTool>,
    /// Agent definitions, such as child agents that `start_task` starts.
    pub agents: Vec<AgentDefinition>,
    /// Instruction sections for every run.
    pub sections: Vec<InstructionSection>,
    /// Task executors, whose kind is the plugin's ID (ADR-0031, decision 4).
    pub executors: Vec<Arc<dyn TaskExecutor>>,
}

/// Why the runtime did not take a plugin's contributions. It took none of
/// them.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContributionError {
    /// The plugin's contributions are registered already.
    #[error("the plugin {plugin} is registered already")]
    PluginRegistered {
        /// The plugin.
        plugin: String,
    },
    /// The core or another plugin has a tool of this name.
    #[error("a tool named {name} is registered already")]
    DuplicateTool {
        /// The tool's name.
        name: String,
    },
    /// The core or another plugin has an agent of this name.
    #[error("an agent named {name} is registered already")]
    DuplicateAgent {
        /// The agent's name.
        name: String,
    },
    /// The core or another plugin has a section with this ID.
    #[error("a section with the ID {id} is registered already")]
    DuplicateSection {
        /// The section's ID.
        id: String,
    },
    /// An executor's kind is not the plugin's ID, or another executor of the
    /// kind is registered.
    #[error("the plugin {plugin} cannot register an executor of the kind {kind}")]
    ExecutorKind {
        /// The plugin.
        plugin: String,
        /// The executor's kind.
        kind: String,
    },
}

/// The contributions of one plugin that is on, without its executors, which
/// the registry of executors keeps.
#[derive(Debug)]
struct Registered {
    plugin: String,
    tools: Vec<PluginTool>,
    agents: Vec<AgentDefinition>,
    sections: Vec<InstructionSection>,
}

/// The contributions of the plugins that are on.
#[derive(Debug, Default)]
pub(crate) struct Plugins {
    registered: RwLock<Vec<Registered>>,
    /// The plugin of each agent that a plugin registered since Mannie
    /// started, kept after the plugin stops, so a call that names the agent
    /// of a plugin that is off says which plugin it was.
    agent_plugins: RwLock<HashMap<&'static str, String>>,
}

/// A plugin's tool as a run is offered it.
#[derive(Debug, Clone)]
pub(crate) struct OfferedTool {
    pub(crate) plugin: String,
    pub(crate) tool: Arc<dyn Tool>,
}

impl Runtime {
    /// Takes a plugin's contributions, as the plugin starts (ADR-0030,
    /// decision 6): runs that start from now on are offered its tools,
    /// agents, sections and executors. Takes all of them or none.
    pub fn add_plugin(
        &self,
        plugin: &str,
        contributions: PluginContributions,
    ) -> Result<(), ContributionError> {
        let mut registered = self
            .inner
            .plugins
            .registered
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        if registered.iter().any(|entry| entry.plugin == plugin) {
            return Err(ContributionError::PluginRegistered {
                plugin: plugin.to_owned(),
            });
        }
        self.check_contributions(plugin, &registered, &contributions)?;
        let PluginContributions {
            tools,
            agents,
            sections,
            executors,
        } = contributions;
        for executor in executors {
            // Checked above: the kind is the plugin's and free.
            if let Err(error) = self.add_executor(executor) {
                return Err(ContributionError::ExecutorKind {
                    plugin: plugin.to_owned(),
                    kind: error.kind.to_owned(),
                });
            }
        }
        {
            let mut agent_plugins = self
                .inner
                .plugins
                .agent_plugins
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            for agent in &agents {
                agent_plugins.insert(agent.name, plugin.to_owned());
            }
        }
        tracing::info!(
            plugin,
            tools = tools.len(),
            agents = agents.len(),
            sections = sections.len(),
            "a plugin's contributions were registered"
        );
        registered.push(Registered {
            plugin: plugin.to_owned(),
            tools,
            agents,
            sections,
        });
        Ok(())
    }

    /// Refuses contributions with a name that the core, another plugin or
    /// the contributions themselves already use, and an executor of another
    /// kind than the plugin's ID, before any of them is registered.
    fn check_contributions(
        &self,
        plugin: &str,
        registered: &[Registered],
        contributions: &PluginContributions,
    ) -> Result<(), ContributionError> {
        let PluginContributions {
            tools,
            agents,
            sections,
            executors,
        } = contributions;
        let mut names: Vec<&str> = Vec::new();
        for offered in tools {
            let name = offered.tool.name();
            let taken = self.inner.tools.registry.get(name).is_some()
                || names.contains(&name)
                || registered
                    .iter()
                    .flat_map(|entry| &entry.tools)
                    .any(|other| other.tool.name() == name);
            if taken {
                return Err(ContributionError::DuplicateTool {
                    name: name.to_owned(),
                });
            }
            names.push(name);
        }
        let mut agent_names: Vec<&str> = Vec::new();
        for agent in agents {
            let taken = self.inner.agents.get(agent.name).is_some()
                || agent_names.contains(&agent.name)
                || registered
                    .iter()
                    .flat_map(|entry| &entry.agents)
                    .any(|other| other.name == agent.name);
            if taken {
                return Err(ContributionError::DuplicateAgent {
                    name: agent.name.to_owned(),
                });
            }
            agent_names.push(agent.name);
        }
        let mut ids: Vec<&str> = Vec::new();
        for section in sections {
            let taken = self.inner.instructions.contains(section.id)
                || ids.contains(&section.id)
                || registered
                    .iter()
                    .flat_map(|entry| &entry.sections)
                    .any(|other| other.id == section.id);
            if taken {
                return Err(ContributionError::DuplicateSection {
                    id: section.id.to_owned(),
                });
            }
            ids.push(section.id);
        }
        // A plugin has at most one executor, whose kind is the plugin's ID.
        let kind_refused = executors.iter().enumerate().find(|(index, executor)| {
            *index > 0 || executor.kind() != plugin || self.executor(executor.kind()).is_some()
        });
        if let Some((_index, executor)) = kind_refused {
            return Err(ContributionError::ExecutorKind {
                plugin: plugin.to_owned(),
                kind: executor.kind().to_owned(),
            });
        }
        Ok(())
    }

    /// Drops a plugin's contributions, as the plugin stops (ADR-0030,
    /// decision 6). Runs that started while they were registered keep them,
    /// and their calls into the plugin are refused; each run lets the tasks
    /// of the plugin's executor go. Returns whether the plugin was
    /// registered.
    pub fn remove_plugin(&self, plugin: &str) -> bool {
        let removed = {
            let mut registered = self
                .inner
                .plugins
                .registered
                .write()
                .unwrap_or_else(PoisonError::into_inner);
            let before = registered.len();
            registered.retain(|entry| entry.plugin != plugin);
            before != registered.len()
        };
        let executor = self.remove_executor(plugin).is_some();
        if removed || executor {
            tracing::info!(plugin, "a plugin's contributions were dropped");
        }
        removed || executor
    }

    /// The definition of an agent: the core's or a plugin's that is on.
    pub(crate) fn agent(&self, name: &str) -> Option<AgentDefinition> {
        if let Some(definition) = self.inner.agents.get(name) {
            return Some(definition.clone());
        }
        self.plugin_agents()
            .into_iter()
            .find(|(_plugin, definition)| definition.name == name)
            .map(|(_plugin, definition)| definition)
    }

    /// The plugin that registered an agent since Mannie started, also while
    /// it is off; `None` for the core's agents and unknown names.
    pub(crate) fn agent_plugin(&self, agent: &str) -> Option<String> {
        self.inner
            .plugins
            .agent_plugins
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(agent)
            .cloned()
    }

    /// The agent definitions of the plugins that are on, with each plugin's
    /// ID, in the order the plugins started.
    pub(crate) fn plugin_agents(&self) -> Vec<(String, AgentDefinition)> {
        self.inner
            .plugins
            .registered
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .flat_map(|entry| {
                entry
                    .agents
                    .iter()
                    .map(|definition| (entry.plugin.clone(), definition.clone()))
            })
            .collect()
    }

    /// The plugins' tools that runs of `agent` are offered now.
    pub(crate) fn plugin_tools_for(&self, agent: &str) -> Vec<OfferedTool> {
        self.inner
            .plugins
            .registered
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .flat_map(|entry| {
                entry
                    .tools
                    .iter()
                    .filter(|offered| offered.agents.contains(&agent))
                    .map(|offered| OfferedTool {
                        plugin: entry.plugin.clone(),
                        tool: Arc::clone(&offered.tool),
                    })
            })
            .collect()
    }

    /// The plugins' instruction sections now.
    pub(crate) fn plugin_sections(&self) -> Vec<InstructionSection> {
        self.inner
            .plugins
            .registered
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .flat_map(|entry| entry.sections.iter().cloned())
            .collect()
    }

    /// Whether a plugin is on with a tool of this name.
    pub(crate) fn plugin_has_tool(&self, plugin: &str, tool: &str) -> bool {
        self.inner
            .plugins
            .registered
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|entry| entry.plugin == plugin)
            .flat_map(|entry| &entry.tools)
            .any(|offered| offered.tool.name() == tool)
    }
}
