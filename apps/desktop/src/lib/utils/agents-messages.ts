// The words of the owner's agents (docs/features/custom-agents.md): the names of
// the built-in agents, the tool groups and the templates, which live in the
// catalog (ADR-0019), and why a command of agents failed.
import type { AgentError, AgentField, AgentToolGroupId, AgentView } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import type { Failure } from '#lib/types/calls.js';

/** The key of the built-in agent of a conversation. */
export const DEFAULT_AGENT = 'conversation';

/** The name of an agent, the catalog's for a built-in one. */
export function agentName(agent: Pick<AgentView, 'builtin' | 'draft' | 'key'>): string {
  if (!agent.builtin) {
    return agent.draft.name;
  }
  switch (agent.key) {
    case DEFAULT_AGENT: {
      return m.agents_name_default();
    }
    case 'explore': {
      return m.agents_name_explore();
    }
    case 'general': {
      return m.agents_name_general();
    }
    default: {
      return agent.draft.name;
    }
  }
}

/** What the Agents page says of an agent, the catalog's for a built-in one. */
export function agentDescription(agent: Pick<AgentView, 'builtin' | 'draft' | 'key'>): string {
  if (!agent.builtin) {
    return agent.draft.description;
  }
  switch (agent.key) {
    case DEFAULT_AGENT: {
      return m.agents_description_default();
    }
    case 'explore': {
      return m.agents_description_explore();
    }
    case 'general': {
      return m.agents_description_general();
    }
    default: {
      return agent.draft.description;
    }
  }
}

/** The name of a group of tools. */
export function groupName(group: AgentToolGroupId): string {
  switch (group) {
    case 'change': {
      return m.agents_group_change();
    }
    case 'commands': {
      return m.agents_group_commands();
    }
    case 'plans': {
      return m.agents_group_plans();
    }
    case 'questions': {
      return m.agents_group_questions();
    }
    case 'read': {
      return m.agents_group_read();
    }
    case 'skills': {
      return m.agents_group_skills();
    }
    case 'tasks': {
      return m.agents_group_tasks();
    }
  }
}

/** What a group of tools lets an agent do. */
export function groupDescription(group: AgentToolGroupId): string {
  switch (group) {
    case 'change': {
      return m.agents_group_change_description();
    }
    case 'commands': {
      return m.agents_group_commands_description();
    }
    case 'plans': {
      return m.agents_group_plans_description();
    }
    case 'questions': {
      return m.agents_group_questions_description();
    }
    case 'read': {
      return m.agents_group_read_description();
    }
    case 'skills': {
      return m.agents_group_skills_description();
    }
    case 'tasks': {
      return m.agents_group_tasks_description();
    }
  }
}

/** The name of a starting point of "New agent". */
export function templateName(id: string): string {
  switch (id) {
    case 'child': {
      return m.agents_template_child();
    }
    case 'conversation': {
      return m.agents_template_conversation();
    }
    case 'orchestrator': {
      return m.agents_template_orchestrator();
    }
    case 'plan': {
      return m.agents_template_plan();
    }
    default: {
      return id;
    }
  }
}

function fieldMessage(field: AgentField): string {
  switch (field) {
    case 'children': {
      return m.agents_error_field_children();
    }
    case 'description': {
      return m.agents_error_field_description();
    }
    case 'instructions': {
      return m.agents_error_field_instructions();
    }
    case 'max_tasks':
    case 'max_tool_calls':
    case 'max_tool_rounds': {
      return m.agents_error_field_limits();
    }
    case 'name': {
      return m.agents_error_field_name();
    }
    case 'plugins':
    case 'tools': {
      return m.agents_error_field_tools();
    }
    case 'profile': {
      return m.agents_error_field_profile();
    }
    case 'steps': {
      return m.agents_error_field_steps();
    }
  }
}

/** Why a command of agents failed, and what the owner can do. */
export function agentFailureMessage(failure: Failure<AgentError>): string {
  if (failure.kind === 'ipc') {
    return m.shell_error_ipc_failed();
  }
  const { error } = failure;
  switch (error.code) {
    case 'builtin': {
      return m.agents_error_builtin();
    }
    case 'field_not_valid': {
      return fieldMessage(error.field);
    }
    case 'kind_changed': {
      return m.agents_error_kind_changed();
    }
    case 'name_taken': {
      return m.agents_error_name_taken();
    }
    case 'not_found': {
      return m.agents_error_not_found();
    }
    case 'storage_failed': {
      return m.agents_error_storage_failed();
    }
    case 'store_unavailable': {
      return m.agents_error_store_unavailable();
    }
    case 'too_many': {
      return m.agents_error_too_many({ max: String(error.max) });
    }
  }
}
