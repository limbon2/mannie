// What the owner does to a child agent from its card
// (docs/features/child-agents/child-agent-activity.md): stopping it, which
// ends only that child's task while its parent continues. The core's stop
// works on any conversation, so a child stops like a reply.
import { SvelteMap, SvelteSet } from 'svelte/reactivity';

import { type ChatError, commands, type ConversationId } from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';
import { call } from '#lib/utils/calls.js';

/** The children being stopped, and why a stop failed. */
export class ChildAgentsActions {
  /** Why stopping a child failed, by the child's conversation. */
  readonly stopFailures = new SvelteMap<ConversationId, Failure<ChatError>>();
  /** The children whose stop the core has not answered yet. */
  readonly stopping = new SvelteSet<ConversationId>();

  /** Stops a child's task. */
  async stop(conversationId: ConversationId): Promise<void> {
    if (this.stopping.has(conversationId)) {
      return;
    }
    this.stopping.add(conversationId);
    this.stopFailures.delete(conversationId);
    const outcome = await call(() => commands.stopReply({ conversationId }));
    this.stopping.delete(conversationId);
    if (!outcome.ok) {
      this.stopFailures.set(conversationId, outcome.failure);
    }
  }
}

/** The window's actions on child agents. */
export const childAgentsActions = new ChildAgentsActions();
