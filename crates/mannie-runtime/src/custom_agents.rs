//! The owner's custom agents (ADR-0059; docs/features/custom-agents.md): the
//! definitions that the store keeps, as runs read them, what a definition may
//! name, the starters and the commands of the Agents page. A definition only
//! narrows what its base has, and the policy never reads it, so it grants
//! nothing (T-87).

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::{PoisonError, RwLock};

use mannie_plugin_sdk::points::tools::{ToolEffect, Tools};
use mannie_store::{Change, RuntimeStoreError, StoredAgent};
use mannie_types::agents::{
    AgentDraft, AgentError, AgentField, AgentKind, AgentOptions, AgentPluginOption, AgentStep,
    AgentTemplate, AgentToolGroup, AgentToolGroupId, AgentView, AgentsSettings,
    CUSTOM_AGENT_PREFIX, MAX_CUSTOM_AGENTS, custom_agent_key,
};

use crate::Runtime;
use crate::agents::{
    AgentDefinition, CONVERSATION_AGENT, EXPLORE_AGENT, EXPLORE_TOOLS, GENERAL_AGENT,
    GENERAL_TOOLS, Requester,
};
use crate::plans::SAVE_PLAN;
use crate::questions::ASK_OWNER;
use crate::skills::READ_SKILL;
use crate::tasks::{START_TASK, TASK_TOOLS};
use crate::texts;

/// The tool that moves a conversation into a project, which every conversation
/// agent that may act has.
const MOVE_TO_PROJECT: &str = "move_to_project";

/// The tools of the group that changes files.
const CHANGE_TOOLS: [&str; 6] = [
    "write_file",
    "edit_file",
    "create_directory",
    "copy_path",
    "move_path",
    "delete_path",
];

/// The tools of the group that runs commands.
const COMMAND_TOOLS: [&str; 4] = [
    "run_command",
    "start_command",
    "read_command_output",
    "stop_command",
];

/// The version of the starters that this release writes.
const STARTERS: u32 = 1;

/// The key of the starter that hands work to child agents.
pub const ORCHESTRATOR_KEY: &str = "custom.orchestrator";

/// The key of the starter that saves plans.
pub const PLAN_KEY: &str = "custom.plan";

/// The tools that an agent of this kind may name (ADR-0059, decision 4): a
/// child only those of the general child, because a task's run does not
/// otherwise exclude `move_to_project`.
fn allowed_tools(kind: AgentKind, read_only: bool) -> Vec<&'static str> {
    let mut allowed: Vec<&'static str> = match (kind, read_only) {
        (_, true) => EXPLORE_TOOLS.to_vec(),
        (AgentKind::Child, false) => GENERAL_TOOLS.to_vec(),
        (AgentKind::Conversation, false) => {
            let mut tools = GENERAL_TOOLS.to_vec();
            tools.push(MOVE_TO_PROJECT);
            tools
        }
    };
    if kind == AgentKind::Conversation {
        allowed.push(ASK_OWNER);
        allowed.extend(TASK_TOOLS);
        allowed.push(SAVE_PLAN);
    }
    // Every kind may read skills, which only reads (ADR-0061, decision 10).
    allowed.push(READ_SKILL);
    allowed
}

/// The built-in agent that plugins see an agent as (ADR-0059, decision 3). An
/// agent that only reads is `explore`, also when it answers to the owner, so it
/// never reads a section that says it may change things.
fn base_of(kind: AgentKind, read_only: bool) -> &'static str {
    match (kind, read_only) {
        (_, true) => EXPLORE_AGENT,
        (AgentKind::Conversation, false) => CONVERSATION_AGENT,
        (AgentKind::Child, false) => GENERAL_AGENT,
    }
}

/// The definition of a stored agent. `custom_children` are the keys of the
/// owner's child agents, so a step that names one that was deleted reads as work
/// for the agent itself.
fn definition_of(stored: &StoredAgent, custom_children: &[String]) -> AgentDefinition {
    let draft = &stored.draft;
    let mut tools: Vec<&'static str> = allowed_tools(draft.kind, draft.read_only)
        .into_iter()
        .filter(|allowed| draft.tools.iter().any(|name| name == allowed))
        .collect();
    let moves = draft.kind == AgentKind::Conversation && !draft.read_only;
    if moves && !tools.contains(&MOVE_TO_PROJECT) {
        tools.push(MOVE_TO_PROJECT);
    }
    // A run is offered the executors' tools by its task budget, and not by its
    // list of tools, so an agent without the task tools has no budget.
    let gives_tasks = tools.contains(&START_TASK);
    if gives_tasks {
        for tool in TASK_TOOLS {
            if !tools.contains(&tool) {
                tools.push(tool);
            }
        }
    }
    let requester = match draft.kind {
        AgentKind::Conversation => Requester::Owner,
        AgentKind::Child => Requester::Agent,
    };
    AgentDefinition {
        name: Cow::Owned(stored.key.clone()),
        base: base_of(draft.kind, draft.read_only),
        requester,
        read_only: draft.read_only,
        description: Cow::Owned(draft.description.clone()),
        instructions: Cow::Owned(instructions_of(draft, &tools, custom_children)),
        tools: Cow::Owned(tools),
        plugins: draft.plugins.clone(),
        children: draft.children.clone(),
        profile: draft.profile_id.clone(),
        max_continuation_notes: 2,
        max_tool_rounds: draft.max_tool_rounds,
        max_tool_calls: draft.max_tool_calls,
        max_tasks: if gives_tasks { draft.max_tasks } else { 0 },
    }
}

/// The owner's instructions, then the steps as the model reads them.
fn instructions_of(draft: &AgentDraft, tools: &[&str], custom_children: &[String]) -> String {
    let gives_tasks = tools.contains(&START_TASK);
    let asks = tools.contains(&ASK_OWNER);
    let stages: Vec<texts::Stage<'_>> = draft
        .steps
        .iter()
        .map(|step| texts::Stage {
            title: &step.title,
            instruction: &step.instruction,
            agent: step
                .agent
                .as_deref()
                .filter(|agent| gives_tasks && step_agent_exists(agent, custom_children)),
            several: step.several,
            ask_first: step.ask_first && asks,
        })
        .collect();
    texts::agent_instructions(&draft.instructions, &stages)
}

fn step_agent_exists(agent: &str, custom_children: &[String]) -> bool {
    !agent.starts_with(CUSTOM_AGENT_PREFIX) || custom_children.iter().any(|child| child == agent)
}

/// The definitions of the stored agents.
fn definitions_of(stored: &[StoredAgent]) -> Vec<AgentDefinition> {
    let custom_children: Vec<String> = stored
        .iter()
        .filter(|agent| agent.draft.kind == AgentKind::Child)
        .map(|agent| agent.key.clone())
        .collect();
    stored
        .iter()
        .map(|agent| definition_of(agent, &custom_children))
        .collect()
}

/// The owner's agents, as runs read them.
#[derive(Debug, Default)]
pub(crate) struct CustomAgents {
    definitions: RwLock<Vec<AgentDefinition>>,
    /// The agents as the store keeps them, for the Agents page.
    stored: RwLock<Vec<StoredAgent>>,
    /// One change of agents at a time, so a name stays unique.
    changes: tokio::sync::Mutex<()>,
}

impl CustomAgents {
    fn replace(&self, stored: Vec<StoredAgent>) {
        let definitions = definitions_of(&stored);
        *self
            .definitions
            .write()
            .unwrap_or_else(PoisonError::into_inner) = definitions;
        *self.stored.write().unwrap_or_else(PoisonError::into_inner) = stored;
    }

    /// The definition with this key.
    pub(crate) fn get(&self, key: &str) -> Option<AgentDefinition> {
        self.definitions
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|definition| definition.name == key)
            .cloned()
    }

    /// The keys of the owner's child agents.
    pub(crate) fn child_keys(&self) -> Vec<String> {
        self.definitions
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|definition| definition.requester == Requester::Agent)
            .map(|definition| definition.name.clone().into_owned())
            .collect()
    }

    fn stored(&self) -> Vec<StoredAgent> {
        self.stored
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The groups of the core's tools that the editor shows.
fn tool_groups() -> Vec<AgentToolGroup> {
    let names = |tools: &[&str]| tools.iter().map(|tool| (*tool).to_owned()).collect();
    vec![
        AgentToolGroup {
            id: AgentToolGroupId::Read,
            tools: names(&EXPLORE_TOOLS),
            for_conversation: true,
            for_child: true,
            for_read_only: true,
        },
        AgentToolGroup {
            id: AgentToolGroupId::Change,
            tools: names(&CHANGE_TOOLS),
            for_conversation: true,
            for_child: true,
            for_read_only: false,
        },
        AgentToolGroup {
            id: AgentToolGroupId::Commands,
            tools: names(&COMMAND_TOOLS),
            for_conversation: true,
            for_child: true,
            for_read_only: false,
        },
        AgentToolGroup {
            id: AgentToolGroupId::Questions,
            tools: names(&[ASK_OWNER]),
            for_conversation: true,
            for_child: false,
            for_read_only: true,
        },
        AgentToolGroup {
            id: AgentToolGroupId::Tasks,
            tools: names(&TASK_TOOLS),
            for_conversation: true,
            for_child: false,
            for_read_only: true,
        },
        AgentToolGroup {
            id: AgentToolGroupId::Plans,
            tools: names(&[SAVE_PLAN]),
            for_conversation: true,
            for_child: false,
            for_read_only: true,
        },
        AgentToolGroup {
            id: AgentToolGroupId::Skills,
            tools: names(&[READ_SKILL]),
            for_conversation: true,
            for_child: true,
            for_read_only: true,
        },
    ]
}

fn owned_names(groups: &[&[&str]]) -> Vec<String> {
    groups
        .iter()
        .flat_map(|group| group.iter().map(|tool| (*tool).to_owned()))
        .collect()
}

/// A blank agent of this kind, with what the built-in agents have.
fn blank(kind: AgentKind) -> AgentDraft {
    let tools = match kind {
        AgentKind::Conversation => owned_names(&[
            &EXPLORE_TOOLS,
            &CHANGE_TOOLS,
            &COMMAND_TOOLS,
            &[ASK_OWNER],
            &TASK_TOOLS,
            &[READ_SKILL],
        ]),
        AgentKind::Child => {
            owned_names(&[&EXPLORE_TOOLS, &CHANGE_TOOLS, &COMMAND_TOOLS, &[READ_SKILL]])
        }
    };
    AgentDraft {
        name: String::new(),
        description: String::new(),
        kind,
        read_only: false,
        profile_id: None,
        instructions: String::new(),
        steps: Vec::new(),
        tools,
        plugins: None,
        children: None,
        max_tasks: match kind {
            AgentKind::Conversation => 20,
            AgentKind::Child => 0,
        },
        max_tool_rounds: 100,
        max_tool_calls: 250,
    }
}

/// The starter that hands work to child agents. It has no plugins' tools, so
/// it has nothing that acts by itself, and it still gives tasks to workers.
fn orchestrator() -> AgentDraft {
    let step = |title: &str, instruction: &str, agent: &str, several: bool| AgentStep {
        title: title.to_owned(),
        instruction: instruction.to_owned(),
        agent: Some(agent.to_owned()),
        several,
        ask_first: false,
    };
    AgentDraft {
        name: texts::ORCHESTRATOR_NAME.to_owned(),
        description: texts::ORCHESTRATOR_DESCRIPTION.to_owned(),
        kind: AgentKind::Conversation,
        read_only: false,
        profile_id: None,
        instructions: texts::ORCHESTRATOR_INSTRUCTIONS.to_owned(),
        steps: vec![
            step(
                texts::ORCHESTRATOR_LOOK_TITLE,
                texts::ORCHESTRATOR_LOOK_TEXT,
                EXPLORE_AGENT,
                true,
            ),
            step(
                texts::ORCHESTRATOR_CHANGE_TITLE,
                texts::ORCHESTRATOR_CHANGE_TEXT,
                GENERAL_AGENT,
                true,
            ),
            step(
                texts::ORCHESTRATOR_CHECK_TITLE,
                texts::ORCHESTRATOR_CHECK_TEXT,
                GENERAL_AGENT,
                false,
            ),
        ],
        tools: owned_names(&[&EXPLORE_TOOLS, &[ASK_OWNER], &TASK_TOOLS, &[READ_SKILL]]),
        plugins: Some(Vec::new()),
        children: None,
        max_tasks: 50,
        max_tool_rounds: 100,
        max_tool_calls: 250,
    }
}

/// The starter that works read-only and saves a plan.
fn plan() -> AgentDraft {
    AgentDraft {
        name: texts::PLAN_NAME.to_owned(),
        description: texts::PLAN_DESCRIPTION.to_owned(),
        kind: AgentKind::Conversation,
        read_only: true,
        profile_id: None,
        instructions: texts::PLAN_INSTRUCTIONS.to_owned(),
        steps: Vec::new(),
        tools: owned_names(&[
            &EXPLORE_TOOLS,
            &[ASK_OWNER],
            &TASK_TOOLS,
            &[SAVE_PLAN],
            &[READ_SKILL],
        ]),
        plugins: None,
        children: None,
        max_tasks: 20,
        max_tool_rounds: 100,
        max_tool_calls: 250,
    }
}

/// A built-in or a plugin's agent, as the Agents page shows it. Its name is its
/// key here, and the UI names the built-in ones from its catalog.
fn builtin_view(definition: &AgentDefinition) -> AgentView {
    let kind = match definition.requester {
        Requester::Owner => AgentKind::Conversation,
        Requester::Agent => AgentKind::Child,
    };
    AgentView {
        key: definition.name.clone().into_owned(),
        builtin: true,
        draft: AgentDraft {
            name: definition.name.clone().into_owned(),
            description: definition.description.clone().into_owned(),
            kind,
            read_only: definition.read_only,
            profile_id: None,
            instructions: String::new(),
            steps: Vec::new(),
            tools: definition
                .tools
                .iter()
                .filter(|tool| **tool != MOVE_TO_PROJECT)
                .map(|tool| (*tool).to_owned())
                .collect(),
            plugins: definition.plugins.clone(),
            children: definition.children.clone(),
            max_tasks: definition.max_tasks,
            max_tool_rounds: definition.max_tool_rounds,
            max_tool_calls: definition.max_tool_calls,
        },
        updated_at: None,
    }
}

fn store_failure(error: &RuntimeStoreError) -> AgentError {
    match error {
        RuntimeStoreError::AgentNotFound => AgentError::NotFound,
        other => {
            tracing::error!(error = %other, error.code = other.code(), "the owner's agents could not be read or saved");
            AgentError::StorageFailed
        }
    }
}

/// What makes two names the same for the owner.
fn same_name(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}

impl Runtime {
    /// Reads the owner's agents from the store into memory.
    pub(crate) async fn load_custom_agents(&self) -> Result<(), RuntimeStoreError> {
        let stored = self.inner.store.custom_agents().await?;
        self.inner.custom_agents.replace(stored);
        Ok(())
    }

    /// The agents of the Agents page: the built-in ones, those of the plugins
    /// that are on, then the owner's, the oldest first.
    pub fn list_agents(&self) -> Vec<AgentView> {
        let snapshot = self.contributions();
        let mut views: Vec<AgentView> = self
            .inner
            .agents
            .definitions()
            .map(builtin_view)
            .chain(
                snapshot
                    .items::<mannie_plugin_sdk::points::agents::Agents>()
                    .iter()
                    .map(|registered| builtin_view(&AgentDefinition::from(&registered.entry))),
            )
            .collect();
        views.extend(
            self.inner
                .custom_agents
                .stored()
                .into_iter()
                .map(|agent| AgentView {
                    key: agent.key,
                    builtin: false,
                    draft: agent.draft,
                    updated_at: Some(agent.updated_at),
                }),
        );
        views
    }

    /// What the editor offers.
    pub fn agent_options(&self) -> AgentOptions {
        let snapshot = self.contributions();
        let mut plugins: BTreeMap<String, AgentPluginOption> = BTreeMap::new();
        let mut add = |plugin: &str, tool: &str, effect: ToolEffect| {
            let option = plugins
                .entry(plugin.to_owned())
                .or_insert_with(|| AgentPluginOption {
                    id: plugin.to_owned(),
                    tools: Vec::new(),
                    has_read_only_tools: false,
                });
            option.tools.push(tool.to_owned());
            option.has_read_only_tools |= effect != ToolEffect::SideEffects;
        };
        for registered in snapshot.items::<Tools>() {
            add(
                registered.plugin.as_str(),
                &registered.entry.spec.name,
                registered.entry.effect,
            );
        }
        for set in snapshot.published() {
            for tool in set.tools.iter() {
                add(set.plugin.as_str(), &tool.spec.name, tool.effect);
            }
        }
        let template = |id: &str, draft| AgentTemplate {
            id: id.to_owned(),
            draft,
        };
        AgentOptions {
            groups: tool_groups(),
            plugins: plugins.into_values().collect(),
            templates: vec![
                template("conversation", blank(AgentKind::Conversation)),
                template("child", blank(AgentKind::Child)),
                template("orchestrator", orchestrator()),
                template("plan", plan()),
            ],
        }
    }

    /// Whether another agent has this name, the built-in ones by their keys and
    /// the names that the UI gives them.
    fn name_is_taken(&self, name: &str, except: Option<&str>) -> bool {
        let builtin = [CONVERSATION_AGENT, "default", EXPLORE_AGENT, GENERAL_AGENT];
        builtin.iter().any(|taken| same_name(taken, name))
            || self
                .inner
                .custom_agents
                .stored()
                .iter()
                .filter(|agent| Some(agent.key.as_str()) != except)
                .any(|agent| same_name(&agent.draft.name, name))
    }

    /// Checks a draft against what its kind may name and what exists (ADR-0059,
    /// decision 4), and returns it cleaned. Nothing here is offered to a run
    /// without `setup_for` applying it again.
    async fn checked_draft(&self, draft: &AgentDraft) -> Result<AgentDraft, AgentError> {
        let draft = draft.check_fields()?;
        let invalid = |field| AgentError::FieldNotValid { field };
        let allowed = allowed_tools(draft.kind, draft.read_only);
        if draft
            .tools
            .iter()
            .any(|tool| !allowed.contains(&tool.as_str()))
        {
            return Err(invalid(AgentField::Tools));
        }
        self.check_children(&draft)?;
        if let Some(profile) = &draft.profile_id {
            let found = self.inner.store.profile(profile).await.map_err(|error| {
                tracing::error!(%error, error.code = error.code(), "a provider profile could not be read");
                AgentError::StorageFailed
            })?;
            if found.is_none() {
                return Err(invalid(AgentField::Profile));
            }
        }
        Ok(draft)
    }

    /// Checks the child agents that a draft and its steps name: each exists, a
    /// child names none, and one that only reads names only those that only
    /// read.
    fn check_children(&self, draft: &AgentDraft) -> Result<(), AgentError> {
        let snapshot = self.contributions();
        let known = self.child_agent_names(&snapshot);
        let usable = |child: &str| {
            known.iter().any(|name| name == child)
                && (!draft.read_only
                    || self
                        .agent_in(&snapshot, child)
                        .is_some_and(|definition| definition.read_only))
        };
        let gives_tasks = draft.tools.iter().any(|tool| tool == START_TASK);
        if let Some(children) = &draft.children {
            let is_valid =
                draft.kind == AgentKind::Conversation && children.iter().all(|child| usable(child));
            if !is_valid {
                return Err(AgentError::FieldNotValid {
                    field: AgentField::Children,
                });
            }
        }
        for step in &draft.steps {
            let Some(child) = &step.agent else {
                continue;
            };
            let in_roster = draft
                .children
                .as_ref()
                .is_none_or(|children| children.contains(child));
            if !(gives_tasks && in_roster && usable(child)) {
                return Err(AgentError::FieldNotValid {
                    field: AgentField::Steps,
                });
            }
        }
        Ok(())
    }

    /// Adds an agent of the owner's.
    pub async fn create_agent(&self, draft: &AgentDraft) -> Result<AgentView, AgentError> {
        let _one_at_a_time = self.inner.custom_agents.changes.lock().await;
        let draft = self.checked_draft(draft).await?;
        let stored = self.inner.custom_agents.stored();
        if stored.len() >= MAX_CUSTOM_AGENTS {
            return Err(AgentError::TooMany {
                max: u32::try_from(MAX_CUSTOM_AGENTS).unwrap_or(u32::MAX),
            });
        }
        if self.name_is_taken(&draft.name, None) {
            return Err(AgentError::NameTaken);
        }
        let key = custom_agent_key(&draft.name, |candidate| {
            stored.iter().any(|agent| agent.key == candidate)
        });
        self.inner
            .store
            .create_custom_agent(&key, &draft)
            .await
            .map_err(|error| store_failure(&error))?;
        self.after_change(&key).await
    }

    /// Changes an agent of the owner's. Its key and its kind stay. A reply that
    /// runs keeps what it started with.
    pub async fn update_agent(
        &self,
        key: &str,
        draft: &AgentDraft,
    ) -> Result<AgentView, AgentError> {
        let _one_at_a_time = self.inner.custom_agents.changes.lock().await;
        let current = self.stored_agent(key)?;
        let draft = self.checked_draft(draft).await?;
        if draft.kind != current.draft.kind {
            return Err(AgentError::KindChanged);
        }
        if self.name_is_taken(&draft.name, Some(key)) {
            return Err(AgentError::NameTaken);
        }
        self.inner
            .store
            .update_custom_agent(key, &draft)
            .await
            .map_err(|error| store_failure(&error))?;
        self.after_change(key).await
    }

    /// Deletes an agent of the owner's, and sets the conversations that use it
    /// back to the built-in agent, each through its hub so every window hears
    /// of it. A conversation that could not be set back reads the deleted key
    /// as the built-in agent anyway.
    pub async fn delete_agent(&self, key: &str) -> Result<(), AgentError> {
        let _one_at_a_time = self.inner.custom_agents.changes.lock().await;
        self.stored_agent(key)?;
        let conversations = self
            .inner
            .store
            .delete_custom_agent(key)
            .await
            .map_err(|error| store_failure(&error))?;
        self.load_custom_agents()
            .await
            .map_err(|error| store_failure(&error))?;
        for conversation in &conversations {
            let set_back = self
                .inner
                .hubs
                .apply(
                    &self.inner.store,
                    conversation,
                    vec![Change::SetAgent { agent_key: None }],
                    mannie_store::Durability::Full,
                )
                .await;
            if let Err(error) = set_back {
                tracing::warn!(%error, error.code = error.code(), conversation = %conversation, "could not set a conversation back to the built-in agent");
            }
        }
        tracing::info!(
            agent = key,
            conversations = conversations.len(),
            "deleted an agent of the owner's"
        );
        Ok(())
    }

    /// The stored agent with this key. A built-in agent's key says so, because
    /// the owner cannot change it.
    fn stored_agent(&self, key: &str) -> Result<StoredAgent, AgentError> {
        if !key.starts_with(CUSTOM_AGENT_PREFIX) {
            return Err(if self.agent(key).is_some() {
                AgentError::Builtin
            } else {
                AgentError::NotFound
            });
        }
        self.inner
            .custom_agents
            .stored()
            .into_iter()
            .find(|agent| agent.key == key)
            .ok_or(AgentError::NotFound)
    }

    /// Reads the agents again after a change, and returns the changed one.
    async fn after_change(&self, key: &str) -> Result<AgentView, AgentError> {
        self.load_custom_agents()
            .await
            .map_err(|error| store_failure(&error))?;
        let agent = self.stored_agent(key)?;
        tracing::info!(agent = key, "saved an agent of the owner's");
        Ok(AgentView {
            key: agent.key,
            builtin: false,
            draft: agent.draft,
            updated_at: Some(agent.updated_at),
        })
    }

    /// Writes the starters once, so one that the owner deletes does not come
    /// back (ADR-0059, decision 14). Only the desktop calls it, and a failure
    /// leaves the app working, and tries again at the next start.
    pub async fn write_starter_agents(&self) -> Result<(), AgentError> {
        let _one_at_a_time = self.inner.custom_agents.changes.lock().await;
        let settings = self
            .inner
            .store
            .load_settings::<AgentsSettings>()
            .await
            .map_err(|error| {
                tracing::warn!(%error, error.code = error.code(), "could not read which starters were written");
                AgentError::StorageFailed
            })?;
        if settings.starters_written >= STARTERS {
            return Ok(());
        }
        let stored = self.inner.custom_agents.stored();
        for (key, draft) in [(ORCHESTRATOR_KEY, orchestrator()), (PLAN_KEY, plan())] {
            if stored.iter().any(|agent| agent.key == key) {
                continue;
            }
            self.inner
                .store
                .create_custom_agent(key, &draft)
                .await
                .map_err(|error| store_failure(&error))?;
        }
        self.inner
            .store
            .save_settings(&AgentsSettings {
                starters_written: STARTERS,
            })
            .await
            .map_err(|error| {
                tracing::warn!(%error, error.code = error.code(), "could not note that the starters were written");
                AgentError::StorageFailed
            })?;
        self.load_custom_agents()
            .await
            .map_err(|error| store_failure(&error))
    }
}

#[cfg(test)]
mod tests {
    use mannie_tools::{PermissionClass, ToolRegistry};
    use mannie_types::clock::UtcMillis;

    use super::*;

    fn stored(key: &str, draft: AgentDraft) -> StoredAgent {
        StoredAgent {
            key: key.to_owned(),
            draft,
            created_at: UtcMillis::EPOCH,
            updated_at: UtcMillis::EPOCH,
        }
    }

    #[test]
    fn each_kind_has_its_base() {
        assert_eq!(base_of(AgentKind::Conversation, false), "conversation");
        assert_eq!(base_of(AgentKind::Conversation, true), "explore");
        assert_eq!(base_of(AgentKind::Child, false), "general");
        assert_eq!(base_of(AgentKind::Child, true), "explore");
    }

    #[test]
    fn a_child_cannot_name_what_only_a_conversation_agent_has() {
        let allowed = allowed_tools(AgentKind::Child, false);
        for tool in [MOVE_TO_PROJECT, ASK_OWNER, SAVE_PLAN, START_TASK] {
            assert!(!allowed.contains(&tool), "{tool}");
        }
    }

    #[test]
    fn what_only_reads_has_no_tool_that_acts() {
        let registry = ToolRegistry::with_system_tools();
        for kind in [AgentKind::Conversation, AgentKind::Child] {
            for tool in allowed_tools(kind, true) {
                if let Some(tool) = registry.get(tool) {
                    assert_eq!(tool.class(), PermissionClass::ReadOnly, "{}", tool.name());
                }
            }
        }
    }

    #[test]
    fn a_stored_name_outside_the_list_is_dropped() {
        let mut draft = blank(AgentKind::Child);
        draft.tools = vec![
            "read_file".to_owned(),
            MOVE_TO_PROJECT.to_owned(),
            "made_up".to_owned(),
        ];

        let definition = definition_of(&stored("custom.a", draft), &[]);

        assert_eq!(definition.tools.as_ref(), ["read_file"]);
    }

    #[test]
    fn a_read_only_definition_drops_what_acts_even_if_stored() {
        let mut draft = blank(AgentKind::Conversation);
        draft.read_only = true;
        draft.tools = vec!["read_file".to_owned(), "write_file".to_owned()];

        let definition = definition_of(&stored("custom.a", draft), &[]);

        assert_eq!(definition.tools.as_ref(), ["read_file"]);
        assert_eq!(definition.base, "explore");
        assert!(definition.read_only);
    }

    #[test]
    fn an_agent_without_the_task_tools_has_no_task_budget() {
        let mut draft = blank(AgentKind::Conversation);
        draft.tools = vec!["read_file".to_owned()];
        draft.max_tasks = 50;

        let definition = definition_of(&stored("custom.a", draft), &[]);

        assert_eq!(definition.max_tasks, 0);
    }

    #[test]
    fn the_task_tools_come_together() {
        let mut draft = blank(AgentKind::Conversation);
        draft.tools = vec![START_TASK.to_owned()];

        let definition = definition_of(&stored("custom.a", draft), &[]);

        for tool in TASK_TOOLS {
            assert!(definition.tools.contains(&tool), "{tool}");
        }
    }

    #[test]
    fn a_step_for_a_deleted_child_reads_as_work_for_the_agent() {
        let mut draft = blank(AgentKind::Conversation);
        draft.steps = vec![AgentStep {
            title: "Review".to_owned(),
            instruction: "Read the changes.".to_owned(),
            agent: Some("custom.reviewer".to_owned()),
            several: false,
            ask_first: false,
        }];
        let agent = stored("custom.a", draft);

        let missing = definition_of(&agent, &[]);
        let there = definition_of(&agent, &["custom.reviewer".to_owned()]);

        assert!(missing.instructions.contains("Do this yourself."));
        assert!(there.instructions.contains("custom.reviewer"));
    }

    #[test]
    fn the_orchestrator_has_nothing_that_acts_by_itself() {
        let definition = definition_of(&stored(ORCHESTRATOR_KEY, orchestrator()), &[]);

        for tool in CHANGE_TOOLS.iter().chain(&COMMAND_TOOLS) {
            assert!(!definition.tools.contains(tool), "{tool}");
        }
        assert_eq!(definition.plugins, Some(Vec::new()));
        assert_eq!(definition.max_tasks, 50);
        for tool in TASK_TOOLS {
            assert!(definition.tools.contains(&tool), "{tool}");
        }
    }

    #[test]
    fn the_plan_agent_only_reads_and_saves() {
        let definition = definition_of(&stored(PLAN_KEY, plan()), &[]);

        assert!(definition.read_only);
        assert_eq!(definition.base, "explore");
        assert!(definition.tools.contains(&SAVE_PLAN));
        assert!(definition.tools.contains(&ASK_OWNER));
        assert!(!definition.tools.contains(&"run_command"));
        assert!(!definition.tools.contains(&MOVE_TO_PROJECT));
    }

    #[test]
    fn the_starters_pass_the_checks_of_their_fields() {
        for draft in [orchestrator(), plan()] {
            let allowed = allowed_tools(draft.kind, draft.read_only);
            draft.check_fields().unwrap();
            for tool in &draft.tools {
                assert!(allowed.contains(&tool.as_str()), "{tool}");
            }
        }
    }
}
