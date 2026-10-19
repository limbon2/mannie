//! Agent definitions (docs/architecture/extension-points.md, agent
//! definitions): a plugin's kind of agent, such as a child agent that
//! `start_task` starts, with whom it answers to, its instructions, its tools
//! and its budgets.
//!
//! An agent's name follows the rule `Global`: it is unique among the core's
//! agents and every plugin's (ADR-0034, decision 6).

use serde::{Deserialize, Serialize};

use crate::contributions::{ExtensionPoint, IdRule};

/// The point of plugins' kinds of agents, whose entries are [`AgentSpec`]s
/// named by their agents' names.
#[derive(Debug)]
pub enum Agents {}

impl ExtensionPoint for Agents {
    const NAME: &'static str = "agents";
    const MAJOR: u16 = 1;
    const IDS: IdRule = IdRule::Global;
    type Entry = AgentSpec;

    fn id(entry: &AgentSpec) -> &str {
        entry.name
    }
}

/// Whom an agent answers to, which decides how the text it reads addresses
/// it (docs/features/child-agents/tasks.md, the text the model sees).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Requester {
    /// The owner: the agent of one of the owner's conversations.
    Owner,
    /// Another agent, which gave it a task: a child agent.
    Agent,
}

/// How much an agent may do in one reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct AgentBudgets {
    /// The most continuation notes in one turn (docs/architecture/
    /// agent-runtime.md, model quirks and early stops).
    pub continuation_notes: u32,
    /// The most model calls with tool calls in one reply.
    pub tool_rounds: u32,
    /// The most tool calls in one reply.
    pub tool_calls: u32,
    /// The most tasks that one reply may start or follow up.
    pub tasks: u32,
}

impl AgentBudgets {
    /// The budgets of an agent. The four numbers are easy to swap, so callers
    /// pass named constants.
    pub const fn new(
        continuation_notes: u32,
        tool_rounds: u32,
        tool_calls: u32,
        tasks: u32,
    ) -> Self {
        Self {
            continuation_notes,
            tool_rounds,
            tool_calls,
            tasks,
        }
    }
}

/// A kind of agent that a plugin adds while it is on.
///
/// The texts are `&'static str`, because the runtime keeps the names of the
/// agents that runs can start for the life of the process, and a plugin
/// defines its agents as constants.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AgentSpec {
    /// Its unique name, as runs record it.
    pub name: &'static str,
    /// Whom it answers to. An agent that answers to another agent is a child
    /// agent, which `start_task` offers.
    pub requester: Requester,
    /// Its own instruction section, after the base instructions. Empty for
    /// none.
    pub instructions: &'static str,
    /// The names of the tools it may use: the core's system tools and task
    /// tools. A plugin's tools name the agents they are offered to
    /// themselves.
    pub tools: &'static [&'static str],
    /// How much it may do in one reply.
    pub budgets: AgentBudgets,
}

impl AgentSpec {
    /// A kind of agent. The name and the instructions are easy to swap, so
    /// callers pass named constants.
    pub const fn new(
        name: &'static str,
        requester: Requester,
        instructions: &'static str,
        tools: &'static [&'static str],
        budgets: AgentBudgets,
    ) -> Self {
        Self {
            name,
            requester,
            instructions,
            tools,
            budgets,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentBudgets, AgentSpec, Requester};

    const NAME: &str = "reviewer";
    const INSTRUCTIONS: &str = "Review the change that you are given.";
    const TOOLS: &[&str] = &["read_file", "search_text"];
    const CONTINUATION_NOTES: u32 = 2;
    const TOOL_ROUNDS: u32 = 50;
    const TOOL_CALLS: u32 = 150;
    const TASKS: u32 = 0;

    /// The constructors are `const`, so a plugin defines its agents as
    /// constants, as the core does.
    const REVIEWER: AgentSpec = AgentSpec::new(
        NAME,
        Requester::Agent,
        INSTRUCTIONS,
        TOOLS,
        AgentBudgets::new(CONTINUATION_NOTES, TOOL_ROUNDS, TOOL_CALLS, TASKS),
    );

    #[test]
    fn the_constructors_keep_each_argument_in_its_field() {
        assert_eq!(REVIEWER.name, "reviewer");
        assert_eq!(REVIEWER.requester, Requester::Agent);
        assert_eq!(
            REVIEWER.instructions,
            "Review the change that you are given."
        );
        assert_eq!(REVIEWER.tools, ["read_file", "search_text"]);
        assert_eq!(REVIEWER.budgets.continuation_notes, 2);
        assert_eq!(REVIEWER.budgets.tool_rounds, 50);
        assert_eq!(REVIEWER.budgets.tool_calls, 150);
        assert_eq!(REVIEWER.budgets.tasks, 0);
    }
}
