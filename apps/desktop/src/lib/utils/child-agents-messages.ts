// Messages for the codes and states of the child agents feature (ADR-0019):
// the commands' errors, the states of tasks, the agents' names and why a call
// of a task tool assigned no task. The UI shows the catalog's message for a
// code and never the core's description. Titles, activities, instructions and
// reports are content and never pass through these functions.
import type { TaskRefusal, TaskState, TaskView } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import type { ChildAgentsFailure } from '#lib/types/child-agents.js';

/** What to tell the owner about a command of the child agents feature that failed. */
export function childAgentsFailureMessage(failure: ChildAgentsFailure): string {
  if (failure.kind === 'ipc') {
    return m.shell_error_ipc_failed();
  }
  const { error } = failure;
  switch (error.code) {
    case 'parallel_children_invalid': {
      return m.child_agents_error_parallel_children_invalid({ max: String(error.max), min: String(error.min) });
    }
    case 'storage_failed': {
      return m.child_agents_error_storage_failed();
    }
    case 'store_unavailable': {
      return m.shell_error_store_unavailable();
    }
    case 'written_by_newer_version': {
      return m.child_agents_error_written_by_newer_version();
    }
  }
}

/** A task's state by its name. */
export function taskStateName(state: TaskState): string {
  switch (state) {
    case 'accepted': {
      return m.child_agents_state_accepted();
    }
    case 'canceled': {
      return m.child_agents_state_canceled();
    }
    case 'failed': {
      return m.child_agents_state_failed();
    }
    case 'interrupted': {
      return m.child_agents_state_interrupted();
    }
    case 'reported': {
      return m.child_agents_state_reported();
    }
    case 'running': {
      return m.child_agents_state_running();
    }
  }
}

/** The task's state as its card names it. A child that waits for the owner says so while it runs. */
export function taskStateLabel(task: TaskView): string {
  const waitsForOwner = task.state === 'running' && task.approval !== null;
  return waitsForOwner ? m.child_agents_state_awaiting_approval() : taskStateName(task.state);
}

/** The name of a child's kind of agent. A kind that this version does not know shows by its name. */
export function agentLabel(agent: string): string {
  switch (agent) {
    case 'explore': {
      return m.child_agents_agent_explore();
    }
    case 'general': {
      return m.child_agents_agent_general();
    }
    default: {
      return m.child_agents_agent_other({ agent });
    }
  }
}

/** Why a call of a task tool assigned no task, as a sentence. */
export function taskRefusalMessage(refusal: TaskRefusal): string {
  switch (refusal.reason) {
    case 'budget_used': {
      return m.child_agents_refusal_budget_used();
    }
    case 'not_started': {
      return m.child_agents_refusal_not_started();
    }
    case 'still_running': {
      return m.child_agents_refusal_still_running({ number: String(refusal.number) });
    }
    case 'unknown_task': {
      return m.child_agents_refusal_unknown_task({ number: String(refusal.number) });
    }
  }
}
