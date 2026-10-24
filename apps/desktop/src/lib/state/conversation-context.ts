// What the parts of a transcript know about the conversation that shows them
// (docs/features/child-agents/child-agent-activity.md, the child's
// conversation in a side panel): whether the owner can act on it, its
// background commands, whose state the calls that started them show
// (ADR-0041), and the excerpts of its turns, which its replies cite by links
// (ADR-0049, decision 4). A task conversation, and any conversation in a
// side panel, shows read-only, without Retry and without the message box.
import { createContext } from 'svelte';

import type { BackgroundCommandView } from '#lib/generated/bindings.js';

import type { CitedItem } from './citations.js';

/** The conversation that a transcript shows, as its parts see it. */
export interface ConversationContext {
  /** The conversation's background commands, by number. */
  readonly backgroundCommands: readonly BackgroundCommandView[];
  /** The stored excerpt of a provider's key that the conversation's messages hold, which a reply's cite link names. */
  readonly cited: (provider: string, key: string) => CitedItem | undefined;
  /** Whether the owner cannot act on the conversation here, so its parts offer no Retry. */
  readonly readOnly: boolean;
}

/** Reads, provides and tells apart the conversation context through Svelte's context. */
export const [getConversationContext, setConversationContext, hasConversationContext] =
  createContext<ConversationContext>();
