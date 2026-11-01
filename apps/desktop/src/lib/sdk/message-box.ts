// The message box of the TypeScript plugin SDK, `@mannie/plugin-sdk/message-box`
// (ADR-0054, decision 4): a plugin's view puts a mention of one of its own
// entries into the message box of a conversation that shows, such as a
// command of a terminal. The mention counts as chosen from the "@" list, and
// its provider checks the key again when the message is sent (ADR-0032,
// decision 5).
import type { MentionEntry } from '#lib/generated/bindings.js';
import { type MentionOutcome, messageBoxes } from '#lib/state/message-boxes.svelte.js';

export type { MentionOutcome } from '#lib/state/message-boxes.svelte.js';

/** An entry of a plugin's mention provider, as the "@" list would show it. */
export interface MentionOfEntry {
  /** The entry, with its key, label and token. */
  readonly entry: MentionEntry;
  /** The mention provider's ID: the plugin's ID, or the plugin's ID, a dot and more. */
  readonly provider: string;
}

/**
 * Puts a mention into the message box of a conversation: `added`, `full`
 * when the message already holds as many mentions as a message can carry, or
 * `not_shown` when no message box of the conversation shows.
 */
export function mention(conversationId: string, mentioned: MentionOfEntry): MentionOutcome {
  return messageBoxes.mention(conversationId, { entry: mentioned.entry, provider: mentioned.provider });
}
