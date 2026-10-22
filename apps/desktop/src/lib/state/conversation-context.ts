// What the parts of a transcript know about the conversation that shows them
// (docs/features/child-agents/child-agent-activity.md, the child's
// conversation in a side panel): whether the owner can act on it, and its
// background commands, whose state the calls that started them show
// (ADR-0041). A task conversation, and any conversation in a side panel,
// shows read-only, without Retry and without the message box.
import { createContext } from 'svelte';

import type { BackgroundCommandView } from '#lib/generated/bindings.js';

/** The conversation that a transcript shows, as its parts see it. */
export interface ConversationContext {
  /** The conversation's background commands, by number. */
  readonly backgroundCommands: readonly BackgroundCommandView[];
  /** Whether the owner cannot act on the conversation here, so its parts offer no Retry. */
  readonly readOnly: boolean;
}

/** Reads and provides the conversation context through Svelte's context. */
export const [getConversationContext, setConversationContext] = createContext<ConversationContext>();
