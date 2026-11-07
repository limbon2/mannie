// What starts the agent editor without a stored agent (docs/features/
// custom-agents.md): "New agent" and "Duplicate" fill a form that the Agents
// page shows until it is saved.
import type { AgentDraft, AgentView } from '#lib/generated/bindings.js';
import { m } from '#lib/paraglide/messages.js';
import { agentName } from '#lib/utils/agents-messages.js';
import { openAgents, shownAgentKey } from '#lib/utils/agents-urls.js';

import { AgentForm } from './agent-form.svelte.js';

/** The agents that the owner starts. */
class AgentActions {
  /** The form of an agent that is not saved yet, while the page shows it. */
  newForm: AgentForm | null = $state(null);

  /** A new agent that waits for the owner's answer about the changes of the one that shows. */
  pending: AgentDraft | null = $state(null);

  /**
   * Starts a new agent from a template. When an unsaved new agent holds changes,
   * the editor asks first. A saved agent's changes ask when the address changes,
   * so this starts nothing when the owner stays.
   */
  async start(draft: AgentDraft): Promise<void> {
    if (this.newForm?.dirty ?? false) {
      this.pending = draft;
      return;
    }
    await openAgents();
    if (shownAgentKey() === null) {
      this.newForm = new AgentForm({ draft, key: null });
    }
  }

  /** Starts the new agent that waited for the owner's answer. */
  async startPending(): Promise<void> {
    const draft = this.pending;
    this.pending = null;
    if (draft !== null) {
      this.newForm = null;
      await this.start(draft);
    }
  }

  /** Forgets the new agent that waited for the owner's answer. */
  cancelPending(): void {
    this.pending = null;
  }

  /** Starts a new agent from another agent's values. */
  async duplicate(agent: AgentView): Promise<void> {
    await this.start({
      ...agent.draft,
      name: m.agents_duplicate_name({ name: agentName(agent) }),
    });
  }

  /** Lets go of the form of an agent that is not saved. */
  discard(): void {
    this.newForm = null;
  }
}

/** The window's agents that the owner starts. */
export const agentActions = new AgentActions();
