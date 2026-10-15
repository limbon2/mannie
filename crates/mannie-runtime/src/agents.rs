//! Agent definitions (docs/architecture/extension-points.md, agent
//! definitions): what each kind of agent is, its instructions, its tools and
//! its budgets. A feature that needs a new kind of agent registers a
//! definition; it does not change the loop.

/// The name of the agent of the owner's conversations.
pub const CONVERSATION_AGENT: &str = "conversation";

/// A kind of agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDefinition {
    /// Its unique name, as runs record it.
    pub name: &'static str,
    /// Its own instruction section, after the base instructions. Empty for
    /// none.
    pub instructions: &'static str,
    /// The names of the tools it may use, of those the registry holds.
    pub tools: &'static [&'static str],
    /// The most continuation notes in one turn (docs/architecture/
    /// agent-runtime.md, model quirks and early stops).
    pub max_continuation_notes: u32,
    /// The most model calls with tool calls in one reply.
    pub max_tool_rounds: u32,
    /// The most tool calls in one reply.
    pub max_tool_calls: u32,
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

/// The conversation agent: the base instructions and the system tools, with
/// 100 rounds and 250 calls of tools in one reply (owner decision of
/// 2026-10-14).
pub const CONVERSATION: AgentDefinition = AgentDefinition {
    name: CONVERSATION_AGENT,
    instructions: "",
    tools: &SYSTEM_TOOLS,
    max_continuation_notes: 2,
    max_tool_rounds: 100,
    max_tool_calls: 250,
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

    /// A registry with Mannie's definitions: the conversation agent.
    pub fn with_builtin() -> Self {
        Self {
            definitions: vec![CONVERSATION],
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
}

#[cfg(test)]
mod tests {
    use super::{AgentDefinition, AgentRegistry, CONVERSATION_AGENT, DuplicateAgent};

    #[test]
    fn a_registered_definition_is_found_and_names_are_unique() {
        let mut registry = AgentRegistry::with_builtin();
        let fake = AgentDefinition {
            name: "fake",
            instructions: "Do fake things.",
            tools: &[],
            max_continuation_notes: 0,
            max_tool_rounds: 0,
            max_tool_calls: 0,
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
    }
}
