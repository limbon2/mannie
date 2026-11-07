// The message boxes of open conversations, which views outside them can add
// mentions to (ADR-0054, decision 4), such as a terminal's panel that
// mentions a command. Each conversation's box registers while it shows; when
// two show the same conversation, the one that registered last takes the
// mentions until it goes. A box can also take a text with a mention, such as a
// plan to run, which waits for a box that does not show yet (ADR-0060, decision
// 10).
import type { MentionChoice } from './message-mentions.svelte.js';

/** The target of the box of the new conversation pane. No conversation has this ID. */
export const NEW_CONVERSATION_BOX = 'new-conversation';

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
  /** Adds a text and a mention to the message. The owner's own text stays. */
  readonly fill: (text: string, choice: MentionChoice) => void;
  /** Adds a mention at the end of the message, or says that the message holds the most mentions. */
  readonly mention: (choice: MentionChoice) => Exclude<MentionOutcome, 'not_shown'>;
}

/** A text and a mention that wait for a box to show. */
interface Seed {
  readonly choice: MentionChoice;
  readonly text: string;
}

/** The message boxes that show, by their conversations. */
class MessageBoxes {
  readonly #boxes = new Map<string, MessageBoxEntry[]>();
  readonly #seeds = new Map<string, Seed>();

  /**
   * Puts a text and a mention into the box of a conversation, or of the new
   * conversation pane. When the box does not show yet, they wait for it. Nothing
   * is sent: the owner presses Send.
   */
  fill(target: string, text: string, choice: MentionChoice): void {
    const box = this.#boxes.get(target)?.at(-1);
    if (box === undefined) {
      this.#seeds.set(target, { choice, text });
    } else {
      box.fill(text, choice);
    }
  }

  /** Adds a mention to the box of a conversation that shows. */
  mention(conversationId: string, choice: MentionChoice): MentionOutcome {
    const box = this.#boxes.get(conversationId)?.at(-1);
    return box === undefined ? 'not_shown' : box.mention(choice);
  }

  /** Registers the box of a conversation while it shows; the function returned unregisters it. */
  register(conversationId: string, box: MessageBoxEntry): () => void {
    const boxes = this.#boxes.get(conversationId) ?? [];
    this.#boxes.set(conversationId, [...boxes, box]);
    const seed = this.#seeds.get(conversationId);
    if (seed !== undefined) {
      this.#seeds.delete(conversationId);
      box.fill(seed.text, seed.choice);
    }
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
