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
    /// The names of the tools it may use. M1 has no tools.
    pub tools: &'static [&'static str],
    /// The most continuation notes in one turn (docs/architecture/
    /// agent-runtime.md, model quirks and early stops).
    pub max_continuation_notes: u32,
}

/// The conversation agent of M1: the base instructions, no tools.
pub const CONVERSATION: AgentDefinition = AgentDefinition {
    name: CONVERSATION_AGENT,
    instructions: "",
    tools: &[],
    max_continuation_notes: 2,
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
