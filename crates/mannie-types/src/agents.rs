//! The owner's custom agents (ADR-0059; docs/features/custom-agents.md): what
//! the owner enters for an agent, what the UI shows of one, the settings that
//! say which starters were written, and the errors of the feature's commands.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::chat::MAX_INSTRUCTIONS_CHARACTERS;
use crate::clock::UtcMillis;
use crate::ids::ProfileId;
use crate::labels::clean_label;
use crate::settings::SettingsSection;

/// What the key of every custom agent starts with. The runtime refuses a
/// plugin's agent with this prefix, so a plugin that is turned on later cannot
/// take over a stored agent (ADR-0059, decision 2).
pub const CUSTOM_AGENT_PREFIX: &str = "custom.";

/// The most custom agents that the owner keeps.
pub const MAX_CUSTOM_AGENTS: usize = 100;

/// The most characters of an agent's name.
pub const MAX_AGENT_NAME_CHARS: usize = 60;

/// The most characters of an agent's description.
pub const MAX_AGENT_DESCRIPTION_CHARS: usize = 300;

/// The most characters of an agent's instructions, as of the owner's own.
pub const MAX_AGENT_INSTRUCTIONS_CHARS: usize = MAX_INSTRUCTIONS_CHARACTERS;

/// The most steps of an agent.
pub const MAX_AGENT_STEPS: usize = 20;

/// The most characters of a step's title.
pub const MAX_STEP_TITLE_CHARS: usize = 80;

/// The most characters of a step's text.
pub const MAX_STEP_TEXT_CHARS: usize = 4_096;

/// The most tasks that an agent's reply may start or follow up.
pub const MAX_AGENT_TASKS: u32 = 200;

/// The most model calls with tool calls in one reply of an agent.
pub const MAX_AGENT_TOOL_ROUNDS: u32 = 500;

/// The most tool calls in one reply of an agent.
pub const MAX_AGENT_TOOL_CALLS: u32 = 2_000;

/// The most names in one of a draft's lists of tools, plugins or children.
const MAX_LISTED_NAMES: usize = 128;

/// The most bytes of one name in those lists, which only the core's checks
/// give a meaning.
const MAX_LISTED_NAME_BYTES: usize = 256;

/// Whom an agent answers to, which never changes after it is created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    /// It answers to the owner, in a conversation that chose it.
    Conversation,
    /// It answers to the agent that gives it tasks.
    Child,
}

/// One entry of an agent's list of steps. The model follows the list as
/// instructions, and nothing enforces it (ADR-0059, decision 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentStep {
    /// A few words.
    pub title: String,
    /// What to do.
    pub instruction: String,
    /// The child agent that does it, or `None` for the agent itself.
    pub agent: Option<String>,
    /// Whether several child agents may do it at the same time.
    pub several: bool,
    /// Whether the agent asks the owner before it starts the step.
    pub ask_first: bool,
}

/// What the owner enters for an agent, and the document that the store keeps.
/// Fields that a later version adds read as their defaults here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentDraft {
    /// The name that the owner sees, unique among the agents.
    pub name: String,
    /// What other agents read when they choose a child.
    pub description: String,
    /// Whom it answers to.
    pub kind: AgentKind,
    /// Whether it is offered only what reads.
    pub read_only: bool,
    /// Its own provider profile, or `None` to use the conversation's or the
    /// parent's.
    pub profile_id: Option<ProfileId>,
    /// Its own instructions.
    pub instructions: String,
    /// Its steps, in their order.
    #[serde(default)]
    pub steps: Vec<AgentStep>,
    /// The names of the core's tools that it may use.
    pub tools: Vec<String>,
    /// The plugins whose tools and sections it gets, or `None` for every
    /// plugin.
    pub plugins: Option<Vec<String>>,
    /// The child agents that it may give tasks to, or `None` for every one.
    pub children: Option<Vec<String>>,
    /// The most tasks that one reply may start or follow up.
    pub max_tasks: u32,
    /// The most model calls with tool calls in one reply.
    pub max_tool_rounds: u32,
    /// The most tool calls in one reply.
    pub max_tool_calls: u32,
}

/// A field of an [`AgentDraft`], for the error that refuses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AgentField {
    /// `name`.
    Name,
    /// `description`.
    Description,
    /// `instructions`.
    Instructions,
    /// `steps`.
    Steps,
    /// `tools`.
    Tools,
    /// `plugins`.
    Plugins,
    /// `children`.
    Children,
    /// `profile_id`.
    Profile,
    /// `max_tasks`.
    MaxTasks,
    /// `max_tool_rounds`.
    MaxToolRounds,
    /// `max_tool_calls`.
    MaxToolCalls,
}

impl AgentDraft {
    /// Checks everything that needs no other agent and no list of tools, and
    /// returns the draft with its names and step titles cleaned. The runtime
    /// checks the rest, because the webview is untrusted (ADR-0004).
    pub fn check_fields(&self) -> Result<Self, AgentError> {
        let invalid = |field| AgentError::FieldNotValid { field };
        let name = cleaned_within(&self.name, MAX_AGENT_NAME_CHARS)
            .ok_or_else(|| invalid(AgentField::Name))?;
        let description = checked_description(self)?;
        let has_blank_instructions_over_limit =
            self.instructions.chars().count() > MAX_AGENT_INSTRUCTIONS_CHARS;
        if has_blank_instructions_over_limit {
            return Err(invalid(AgentField::Instructions));
        }
        let steps = checked_steps(&self.steps)?;
        for (names, field) in [
            (Some(&self.tools), AgentField::Tools),
            (self.plugins.as_ref(), AgentField::Plugins),
            (self.children.as_ref(), AgentField::Children),
        ] {
            if names.is_some_and(|names| !names_are_valid(names)) {
                return Err(invalid(field));
            }
        }
        for (value, range, field) in [
            (self.max_tasks, 0..=MAX_AGENT_TASKS, AgentField::MaxTasks),
            (
                self.max_tool_rounds,
                1..=MAX_AGENT_TOOL_ROUNDS,
                AgentField::MaxToolRounds,
            ),
            (
                self.max_tool_calls,
                1..=MAX_AGENT_TOOL_CALLS,
                AgentField::MaxToolCalls,
            ),
        ] {
            if !range.contains(&value) {
                return Err(invalid(field));
            }
        }
        Ok(Self {
            name,
            description,
            steps,
            ..self.clone()
        })
    }
}

/// A text cleaned as a label, when it fits the limit: a text over it is refused
/// rather than cut, so an agent never changes by being cut.
fn cleaned_within(text: &str, max_chars: usize) -> Option<String> {
    if text.chars().count() > max_chars {
        return None;
    }
    clean_label(text, max_chars)
}

/// A child's description is what the agents that give it tasks choose by, so a
/// child needs one, and a conversation agent's is only for the owner.
fn checked_description(draft: &AgentDraft) -> Result<String, AgentError> {
    let invalid = AgentError::FieldNotValid {
        field: AgentField::Description,
    };
    let Some(description) = cleaned_within(&draft.description, MAX_AGENT_DESCRIPTION_CHARS) else {
        let is_blank = draft.description.trim().is_empty();
        let may_be_blank = draft.kind == AgentKind::Conversation;
        return if is_blank && may_be_blank {
            Ok(String::new())
        } else {
            Err(invalid)
        };
    };
    Ok(description)
}

fn checked_steps(steps: &[AgentStep]) -> Result<Vec<AgentStep>, AgentError> {
    let invalid = AgentError::FieldNotValid {
        field: AgentField::Steps,
    };
    if steps.len() > MAX_AGENT_STEPS {
        return Err(invalid);
    }
    steps
        .iter()
        .map(|step| {
            let title = cleaned_within(&step.title, MAX_STEP_TITLE_CHARS);
            let text_is_valid = !step.instruction.trim().is_empty()
                && step.instruction.chars().count() <= MAX_STEP_TEXT_CHARS;
            match title {
                Some(title) if text_is_valid => Ok(AgentStep {
                    title,
                    ..step.clone()
                }),
                _ => Err(AgentError::FieldNotValid {
                    field: AgentField::Steps,
                }),
            }
        })
        .collect()
}

fn names_are_valid(names: &[String]) -> bool {
    names.len() <= MAX_LISTED_NAMES
        && names
            .iter()
            .all(|name| !name.is_empty() && name.len() <= MAX_LISTED_NAME_BYTES)
}

/// The key of a new custom agent with this name, among those taken: its letters
/// and digits in lower case, with a number when the key is taken. A name
/// without ASCII letters or digits gets the key `custom.agent`.
pub fn custom_agent_key(name: &str, is_taken: impl Fn(&str) -> bool) -> String {
    let mut slug = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= MAX_SLUG_BYTES {
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    let slug = if slug.is_empty() { "agent" } else { slug };
    let first = format!("{CUSTOM_AGENT_PREFIX}{slug}");
    if !is_taken(&first) {
        return first;
    }
    (2..=u32::MAX)
        .map(|number| format!("{first}-{number}"))
        .find(|candidate| !is_taken(candidate))
        .unwrap_or(first)
}

/// The most bytes of a key's part after the prefix, before a number.
const MAX_SLUG_BYTES: usize = 40;

/// An agent as the UI shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentView {
    /// Its key, as runs and conversations record it.
    pub key: String,
    /// Whether it is built in or a plugin's, so the owner can only duplicate
    /// it.
    pub builtin: bool,
    /// What it is.
    pub draft: AgentDraft,
    /// When it last changed, for an agent of the owner's.
    pub updated_at: Option<UtcMillis>,
}

/// A group of the core's tools that the editor shows with one checkbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum AgentToolGroupId {
    /// Tools that read and search.
    Read,
    /// Tools that change files.
    Change,
    /// Tools that run commands.
    Commands,
    /// The tool that asks the owner questions.
    Questions,
    /// The tools that give tasks, with the executors of plugins.
    Tasks,
    /// The tool that saves plans.
    Plans,
}

/// A group of the core's tools, with who can have it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolGroup {
    /// The group, whose name the catalog holds.
    pub id: AgentToolGroupId,
    /// Its tools.
    pub tools: Vec<String>,
    /// Whether a conversation agent can have it.
    pub for_conversation: bool,
    /// Whether a child agent can have it.
    pub for_child: bool,
    /// Whether an agent that only reads can have it.
    pub for_read_only: bool,
}

/// A plugin that has tools an agent could get.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentPluginOption {
    /// The plugin.
    pub id: String,
    /// The names of its tools, as they are now.
    pub tools: Vec<String>,
    /// Whether at least one of them only reads.
    pub has_read_only_tools: bool,
}

/// A starting point of "New agent".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentTemplate {
    /// The template, whose name the catalog holds.
    pub id: String,
    /// What the editor starts with.
    pub draft: AgentDraft,
}

/// What the editor offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentOptions {
    /// The groups of the core's tools.
    pub groups: Vec<AgentToolGroup>,
    /// The plugins that are on and have tools.
    pub plugins: Vec<AgentPluginOption>,
    /// The starting points of "New agent".
    pub templates: Vec<AgentTemplate>,
}

/// The argument of `update_agent`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAgent {
    /// The agent.
    pub key: String,
    /// What it is from now on.
    pub draft: AgentDraft,
}

/// The argument of `delete_agent`.
#[derive(Debug, Clone, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentRef {
    /// The agent.
    pub key: String,
}

/// Why a command of the owner's agents failed. The UI shows the message for
/// the code from its catalog (ADR-0019).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(
    tag = "code",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum AgentError {
    /// The agent does not exist.
    #[error("the agent does not exist")]
    NotFound,
    /// Another agent has this name.
    #[error("another agent has this name")]
    NameTaken,
    /// A field of the draft is not valid.
    #[error("a field of the agent is not valid")]
    FieldNotValid {
        /// The field.
        field: AgentField,
    },
    /// The agent is built in or a plugin's, so it cannot change.
    #[error("a built-in agent cannot change")]
    Builtin,
    /// An agent's kind never changes.
    #[error("the kind of an agent cannot change")]
    KindChanged,
    /// The owner keeps as many agents as Mannie allows.
    #[error("the owner keeps as many agents as Mannie allows")]
    TooMany {
        /// The limit.
        max: u32,
    },
    /// The store failed.
    #[error("the agents could not be read or saved")]
    StorageFailed,
    /// The store is not available.
    #[error("the store is not available")]
    StoreUnavailable,
}

impl AgentError {
    /// The stable code of the error, for logs.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "agents.not_found",
            Self::NameTaken => "agents.name_taken",
            Self::FieldNotValid { .. } => "agents.field_not_valid",
            Self::Builtin => "agents.builtin",
            Self::KindChanged => "agents.kind_changed",
            Self::TooMany { .. } => "agents.too_many",
            Self::StorageFailed => "agents.storage_failed",
            Self::StoreUnavailable => "agents.store_unavailable",
        }
    }
}

/// The `agents` settings section: which of the starters the desktop wrote, so
/// a starter that the owner deleted does not come back (ADR-0059, decision 14).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentsSettings {
    /// The version of the starters that were written, 0 for none.
    pub starters_written: u32,
}

impl SettingsSection for AgentsSettings {
    const KEY: &'static str = "agents";
    const VERSION: u32 = 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> AgentDraft {
        AgentDraft {
            name: "  Test   runner ".to_owned(),
            description: "Runs the tests.".to_owned(),
            kind: AgentKind::Child,
            read_only: false,
            profile_id: None,
            instructions: "Run them.".to_owned(),
            steps: vec![AgentStep {
                title: " Run\nthem ".to_owned(),
                instruction: "All of them.".to_owned(),
                agent: None,
                several: false,
                ask_first: false,
            }],
            tools: vec!["read_file".to_owned()],
            plugins: None,
            children: None,
            max_tasks: 0,
            max_tool_rounds: 100,
            max_tool_calls: 250,
        }
    }

    fn refused(draft: &AgentDraft) -> Option<AgentField> {
        match draft.check_fields() {
            Err(AgentError::FieldNotValid { field }) => Some(field),
            _ => None,
        }
    }

    #[test]
    fn a_valid_draft_comes_back_with_its_labels_cleaned() {
        let checked = draft().check_fields().unwrap();

        assert_eq!(checked.name, "Test runner");
        assert_eq!(checked.steps[0].title, "Run them");
    }

    #[test]
    fn each_field_over_its_limit_is_refused_and_not_cut() {
        let mut long_name = draft();
        long_name.name = "n".repeat(MAX_AGENT_NAME_CHARS + 1);
        assert_eq!(refused(&long_name), Some(AgentField::Name));

        let mut blank_name = draft();
        blank_name.name = " \n ".to_owned();
        assert_eq!(refused(&blank_name), Some(AgentField::Name));

        let mut long_description = draft();
        long_description.description = "d".repeat(MAX_AGENT_DESCRIPTION_CHARS + 1);
        assert_eq!(refused(&long_description), Some(AgentField::Description));

        let mut long_instructions = draft();
        long_instructions.instructions = "i".repeat(MAX_AGENT_INSTRUCTIONS_CHARS + 1);
        assert_eq!(refused(&long_instructions), Some(AgentField::Instructions));

        let mut too_many_steps = draft();
        too_many_steps.steps = vec![draft().steps[0].clone(); MAX_AGENT_STEPS + 1];
        assert_eq!(refused(&too_many_steps), Some(AgentField::Steps));

        let mut blank_step = draft();
        blank_step.steps[0].instruction = "  ".to_owned();
        assert_eq!(refused(&blank_step), Some(AgentField::Steps));
    }

    #[test]
    fn a_child_needs_a_description_and_a_conversation_agent_does_not() {
        let mut child = draft();
        child.description = " ".to_owned();
        assert_eq!(refused(&child), Some(AgentField::Description));

        let mut conversation = draft();
        conversation.description = " ".to_owned();
        conversation.kind = AgentKind::Conversation;
        assert_eq!(conversation.check_fields().unwrap().description, "");
    }

    #[test]
    fn each_limit_has_its_range() {
        let mut tasks = draft();
        tasks.max_tasks = MAX_AGENT_TASKS + 1;
        assert_eq!(refused(&tasks), Some(AgentField::MaxTasks));

        let mut rounds = draft();
        rounds.max_tool_rounds = 0;
        assert_eq!(refused(&rounds), Some(AgentField::MaxToolRounds));

        let mut calls = draft();
        calls.max_tool_calls = MAX_AGENT_TOOL_CALLS + 1;
        assert_eq!(refused(&calls), Some(AgentField::MaxToolCalls));
    }

    #[test]
    fn a_list_of_names_is_limited() {
        let mut tools = draft();
        tools.tools = vec!["x".to_owned(); MAX_LISTED_NAMES + 1];
        assert_eq!(refused(&tools), Some(AgentField::Tools));

        let mut plugins = draft();
        plugins.plugins = Some(vec![String::new()]);
        assert_eq!(refused(&plugins), Some(AgentField::Plugins));

        let mut children = draft();
        children.children = Some(vec!["y".repeat(MAX_LISTED_NAME_BYTES + 1)]);
        assert_eq!(refused(&children), Some(AgentField::Children));
    }

    #[test]
    fn a_key_comes_from_the_name_and_is_unique() {
        let none = |_: &str| false;
        assert_eq!(custom_agent_key("Test runner!", none), "custom.test-runner");
        assert_eq!(custom_agent_key("  Plan  ", none), "custom.plan");
        assert_eq!(custom_agent_key("Планировщик", none), "custom.agent");

        let taken = |key: &str| key == "custom.plan" || key == "custom.plan-2";
        assert_eq!(custom_agent_key("Plan", taken), "custom.plan-3");

        let long = custom_agent_key(&"a".repeat(200), none);
        assert_eq!(long.len(), CUSTOM_AGENT_PREFIX.len() + MAX_SLUG_BYTES);
    }

    #[test]
    fn a_stored_draft_from_before_steps_reads_without_any() {
        let stored = serde_json::json!({
            "name": "Old",
            "description": "",
            "kind": "conversation",
            "readOnly": false,
            "profileId": null,
            "instructions": "",
            "tools": [],
            "plugins": null,
            "children": null,
            "maxTasks": 0,
            "maxToolRounds": 1,
            "maxToolCalls": 1,
        });

        let draft: AgentDraft = serde_json::from_value(stored).unwrap();

        assert_eq!(draft.steps, Vec::new());
    }
}
