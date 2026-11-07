// The owner's agents as the UI knows them (docs/features/custom-agents.md). The
// core checks every draft against what its kind may name, and keeps the list.
import {
  type AgentDraft,
  type AgentError,
  type AgentOptions,
  type AgentView,
  commands,
  type UpdateAgent,
} from '#lib/generated/bindings.js';
import type { Failure, Outcome } from '#lib/types/calls.js';
import { call } from '#lib/utils/calls.js';

/** The window's agents. */
export class Agents {
  agents: AgentView[] = $state([]);
  /** Why the agents could not be read, if they could not. */
  failure: Failure<AgentError> | null = $state(null);
  /** What the editor offers, once it was read. */
  options: AgentOptions | null = $state(null);
  status: 'error' | 'loading' | 'ready' = $state('loading');
  #loaded = false;

  /** The agents that a conversation can use, the built-in one first. */
  get conversationAgents(): AgentView[] {
    return this.agents.filter((agent) => agent.draft.kind === 'conversation');
  }

  /** The agents that can be given tasks, the built-in ones first. */
  get childAgents(): AgentView[] {
    return this.agents.filter((agent) => agent.draft.kind === 'child');
  }

  /** Adds an agent and reads the list again. */
  async create(draft: AgentDraft): Promise<Outcome<AgentView, AgentError>> {
    const outcome = await call(() => commands.createAgent(draft));
    await this.reload();
    return outcome;
  }

  /** Deletes an agent and reads the list again. */
  async delete(key: string): Promise<Outcome<null, AgentError>> {
    const outcome = await call(() => commands.deleteAgent({ key }));
    await this.reload();
    return outcome;
  }

  /** Reads the agents the first time something needs them. */
  async ensureLoaded(): Promise<void> {
    if (this.#loaded) {
      return;
    }
    this.#loaded = true;
    await this.reload();
  }

  /** The agent with this key, if it exists. */
  find(key: string | null): AgentView | undefined {
    return key === null ? undefined : this.agents.find((agent) => agent.key === key);
  }

  /** Reads the agents and what the editor offers from the core. */
  async reload(): Promise<void> {
    if (this.agents.length === 0) {
      this.status = 'loading';
    }
    const listed = await call(() => commands.listAgents());
    const offered = await call(() => commands.agentOptions());
    if (listed.ok && offered.ok) {
      this.agents = listed.data;
      this.options = offered.data;
      this.status = 'ready';
      this.failure = null;
      return;
    }
    if (!listed.ok) {
      this.failure = listed.failure;
    } else if (!offered.ok) {
      this.failure = offered.failure;
    }
    this.status = 'error';
  }

  /** Changes an agent and reads the list again. */
  async update(input: UpdateAgent): Promise<Outcome<AgentView, AgentError>> {
    const outcome = await call(() => commands.updateAgent(input));
    await this.reload();
    return outcome;
  }
}

/** The window's agents. */
export const agents = new Agents();
