// The message boxes of open conversations, which views outside them can add
// mentions to (ADR-0054, decision 4), such as a terminal's panel that
// mentions a command. Each conversation's box registers while it shows; when
// two show the same conversation, the one that registered last takes the
// mentions until it goes.
import type { MentionChoice } from './message-mentions.svelte.js';

/** What adding a mention to a message box did. */
export type MentionOutcome =
  /** The mention is in the message, as if the owner had chosen it from the list. */
  | 'added'
  /** The message already holds as many mentions as a message can carry. */
  | 'full'
  /** No message box of that conversation shows. */
  | 'not_shown';

/** A message box that takes mentions from outside it. */
export interface MessageBoxEntry {
  /** Adds a mention at the end of the message, or says that the message holds the most mentions. */
  readonly mention: (choice: MentionChoice) => Exclude<MentionOutcome, 'not_shown'>;
}

/** The message boxes that show, by their conversations. */
class MessageBoxes {
  readonly #boxes = new Map<string, MessageBoxEntry[]>();

  /** Adds a mention to the box of a conversation that shows. */
  mention(conversationId: string, choice: MentionChoice): MentionOutcome {
    const box = this.#boxes.get(conversationId)?.at(-1);
    return box === undefined ? 'not_shown' : box.mention(choice);
  }

  /** Registers the box of a conversation while it shows; the function returned unregisters it. */
  register(conversationId: string, box: MessageBoxEntry): () => void {
    const boxes = this.#boxes.get(conversationId) ?? [];
    this.#boxes.set(conversationId, [...boxes, box]);
    return () => {
      const remaining = (this.#boxes.get(conversationId) ?? []).filter((registered) => registered !== box);
      if (remaining.length === 0) {
        this.#boxes.delete(conversationId);
      } else {
        this.#boxes.set(conversationId, remaining);
      }
    };
  }
}

/** The message boxes of the window. */
export const messageBoxes = new MessageBoxes();
