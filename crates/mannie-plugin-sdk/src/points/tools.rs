//! Tools (docs/architecture/extension-points.md, tool registry): a plugin's
//! tools, which the model calls, each offered to the agents that its
//! selector names, so a plugin does not depend on the core's agent names
//! (ADR-0034, decision 6).
//!
//! The core runs a plugin's tool through its own tool loop (ADR-0013): it
//! prepares the call, which checks the arguments and returns what the call
//! touches, the policy decides on that, the call is journaled, and only then
//! does the tool execute. SDK 1 has no access that a tool may declare
//! ([`ToolAccess`]), so a plugin's tool reads only its own plugin's data and
//! the owner is never asked about it. A tool that acts on files, processes or
//! the network, or takes a path, a shell string or a URL, needs an SDK change
//! with an ADR that maps it onto the core's accesses and policy.
//!
//! A tool's name follows the rule `Global`: it is unique among the core's
//! tools, the task tools and every plugin's tools (decision 6).

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::BoxFuture;
use crate::contracts::ConversationId;
use crate::contributions::{ExtensionPoint, IdRule};

/// The point of plugins' tools, whose entries are [`PluginTool`]s named by
/// their tools' names.
#[derive(Debug)]
pub enum Tools {}

impl ExtensionPoint for Tools {
    const NAME: &'static str = "tools";
    const MAJOR: u16 = 1;
    const IDS: IdRule = IdRule::Global;
    type Entry = PluginTool;

    fn id(entry: &PluginTool) -> &str {
        &entry.spec.name
    }
}

/// A tool that a plugin adds while it is on.
///
/// Both futures own their data, and the core cancels a call by dropping its
/// future (decision 3), as it does when the owner stops the reply.
pub trait Tool: Send + Sync + fmt::Debug + 'static {
    /// What the model is told of the tool. The core reads it once, when the
    /// plugin builds its [`PluginTool`].
    fn spec(&self) -> ToolSpec;

    /// Checks a call's arguments without side effects and returns what the
    /// call touches. A refusal says what is wrong with the arguments, the
    /// model reads it, and the call never runs.
    fn prepare(&self, call: ToolCall) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>>;

    /// Performs a call after the core authorized it. A failure is a result
    /// like any other, whose output tells the model what went wrong.
    fn execute(&self, execution: ToolExecution) -> BoxFuture<'static, ToolResult>;
}

/// What the model is told of a tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ToolSpec {
    /// The tool's name: lower-case snake case of at most 64 characters,
    /// unique among the core's and every plugin's tools, and never reused for
    /// another meaning.
    pub name: String,
    /// What the tool does, for the model.
    pub description: String,
    /// The JSON Schema of the arguments. It has no argument named
    /// `activity`, which the core adds to every call for the owner.
    pub parameters: serde_json::Value,
}

impl ToolSpec {
    /// The spec of a tool, with the JSON Schema of its arguments.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}

/// What a call of a plugin's tool touches, on which the core's policy
/// decides.
///
/// SDK 1 has no access (decision 6), so a value of this type cannot exist.
/// An access to files, processes or remote services comes with the SDK change
/// that maps it onto the core's accesses. The enum stays exhaustive, because
/// plugins produce its values and the core matches them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolAccess {}

/// The agents whose runs are offered a plugin's tool, when their profiles
/// use tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSelector {
    /// The agents that answer to the owner: those of the owner's
    /// conversations.
    Conversations,
    /// The agents that answer to another agent: child agents, the core's and
    /// the plugins'.
    ChildAgents,
    /// Every agent.
    Every,
    /// The agents with these names, the core's or the plugins'.
    Agents(Vec<String>),
}

/// A tool of a plugin, with the agents that it is offered to.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PluginTool {
    /// The tool.
    pub tool: Arc<dyn Tool>,
    /// The agents whose runs are offered the tool.
    pub offered_to: AgentSelector,
    /// What the model is told of the tool, as the tool gave it when the
    /// plugin built this entry.
    pub spec: ToolSpec,
}

impl PluginTool {
    /// A tool for the agents that `offered_to` names. Reads the tool's spec
    /// once, so the model is told the same of the tool while the plugin is
    /// on.
    pub fn new(tool: Arc<dyn Tool>, offered_to: AgentSelector) -> Self {
        let spec = tool.spec();
        Self {
            tool,
            offered_to,
            spec,
        }
    }
}

/// A call of a plugin's tool, as the core prepares it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ToolCall {
    /// The arguments as the model wrote them, as JSON text, without the
    /// call's activity. A model that calls a tool without arguments may send
    /// an empty text or `null`.
    pub arguments: String,
    /// The conversation of the agent that made the call.
    pub conversation_id: ConversationId,
}

impl ToolCall {
    /// A call with its arguments. Only the core builds one.
    #[cfg(feature = "host")]
    pub fn new(arguments: String, conversation_id: ConversationId) -> Self {
        Self {
            arguments,
            conversation_id,
        }
    }
}

/// A call of a plugin's tool that the core authorized and now executes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ToolExecution {
    /// The arguments as the model wrote them, as JSON text, without the
    /// call's activity: the same arguments that the call was prepared with.
    pub arguments: String,
    /// The conversation of the agent that made the call.
    pub conversation_id: ConversationId,
}

impl ToolExecution {
    /// An execution of a call with its arguments. Only the core builds one.
    #[cfg(feature = "host")]
    pub fn new(arguments: String, conversation_id: ConversationId) -> Self {
        Self {
            arguments,
            conversation_id,
        }
    }
}

/// Why a plugin's tool refuses a call's arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ToolRefusal {
    /// What is wrong with the arguments, for the model.
    pub detail: String,
}

impl ToolRefusal {
    /// A refusal that tells the model what is wrong with the arguments.
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

/// What a call of a plugin's tool gives the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ToolResult {
    /// The result as the model reads it. The core cuts a longer output to the
    /// result limit of the run.
    pub output: String,
}

impl ToolResult {
    /// The result of a call, as the model reads it.
    pub fn new(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures_util::FutureExt;
    use serde_json::json;

    use super::{
        AgentSelector, PluginTool, Tool, ToolAccess, ToolCall, ToolExecution, ToolRefusal,
        ToolResult, ToolSpec,
    };
    use crate::BoxFuture;

    /// A tool that counts how often the core asked for its spec.
    #[derive(Debug, Default)]
    struct CountedSpec {
        specs: AtomicUsize,
    }

    impl Tool for CountedSpec {
        fn spec(&self) -> ToolSpec {
            self.specs.fetch_add(1, Ordering::SeqCst);
            ToolSpec::new(
                "notes_find",
                "Finds the owner's notes.",
                json!({ "type": "object", "properties": {} }),
            )
        }

        fn prepare(
            &self,
            _call: ToolCall,
        ) -> BoxFuture<'static, Result<Vec<ToolAccess>, ToolRefusal>> {
            async { Err(ToolRefusal::new("no notes yet")) }.boxed()
        }

        fn execute(&self, _execution: ToolExecution) -> BoxFuture<'static, ToolResult> {
            async { ToolResult::new("No notes.") }.boxed()
        }
    }

    #[test]
    fn a_plugin_tool_reads_its_spec_once_and_keeps_its_selector() {
        let tool = Arc::new(CountedSpec::default());

        let entry = PluginTool::new(
            Arc::<CountedSpec>::clone(&tool),
            AgentSelector::Agents(vec!["explore".to_owned()]),
        );
        let copy = entry.clone();

        // A copy of the entry keeps the spec and asks the tool for none.
        assert_eq!(tool.specs.load(Ordering::SeqCst), 1);
        assert_eq!(copy.spec, entry.spec);
        assert_eq!(entry.spec.name, "notes_find");
        assert_eq!(entry.spec.description, "Finds the owner's notes.");
        assert_eq!(
            entry.spec.parameters,
            json!({ "type": "object", "properties": {} })
        );
        assert_eq!(
            entry.offered_to,
            AgentSelector::Agents(vec!["explore".to_owned()])
        );
    }

    #[test]
    fn the_constructors_keep_each_argument_in_its_field() {
        assert_eq!(
            ToolRefusal::new("the text is empty").detail,
            "the text is empty"
        );
        assert_eq!(ToolResult::new("2 notes.").output, "2 notes.");
    }

    #[cfg(feature = "host")]
    #[test]
    fn the_core_s_constructors_keep_the_arguments_and_the_conversation() {
        use crate::contracts::ConversationId;

        let conversation = ConversationId::parse("0199e0f2-0000-7000-8000-000000000001").unwrap();

        let call = ToolCall::new(r#"{"text":"hi"}"#.to_owned(), conversation.clone());
        let execution = ToolExecution::new(r#"{"text":"hi"}"#.to_owned(), conversation.clone());

        assert_eq!(call.arguments, r#"{"text":"hi"}"#);
        assert_eq!(call.conversation_id, conversation);
        assert_eq!(execution.arguments, r#"{"text":"hi"}"#);
        assert_eq!(execution.conversation_id, conversation);
    }
}
