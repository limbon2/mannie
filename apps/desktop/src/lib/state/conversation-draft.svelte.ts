// The conversation that the first message of the new conversation pane
// starts (ADR-0046, decision 6; ADR-0051): what it starts with attached and
// the attachment that it belongs to, which the controls of the pane's message
// box and a plugin's `startConversation` set. The pane sends them with the
// first message, and the next new conversation starts with nothing attached.
import type { Attachment, AttachmentRef } from '#lib/generated/bindings.js';

/** Whether two attachments name the same key of the same provider. */
export function sameKey(left: AttachmentRef, right: AttachmentRef): boolean {
  const sameProvider = left.provider === right.provider;
  const sameProviderKey = left.key === right.key;
  return sameProvider && sameProviderKey;
}

/** Whether an attachment is the home, when there is one. */
function isHome(home: AttachmentRef | null, attachment: AttachmentRef): boolean {
  if (home === null) {
    return false;
  }
  return sameKey(home, attachment);
}

/** What the new conversation starts with attached. */
export class ConversationDraftState {
  /** The attachments, in the order the owner attached them, the home first. */
  attachments: Attachment[] = $state([]);

  /** The attachment that the conversation belongs to, which stays attached, or `null`. */
  home: AttachmentRef | null = $state(null);

  /** Adds an attachment after the others, or gives one that the draft holds this label. */
  attach(attachment: Attachment): void {
    const index = this.attachments.findIndex((held) => sameKey(held, attachment));
    if (index === -1) {
      this.attachments.push(attachment);
    } else {
      this.attachments[index] = attachment;
    }
  }

  /** Removes an attachment; one that the draft does not hold, or its home, changes nothing. */
  detach(attachment: AttachmentRef): void {
    if (isHome(this.home, attachment)) {
      return;
    }
    this.attachments = this.attachments.filter((held) => !sameKey(held, attachment));
  }

  /** Lets go of the home, so the conversation is one of the Chat's, and keeps the other attachments. */
  leaveHome(): void {
    const home = this.home;
    if (home === null) {
      return;
    }
    this.home = null;
    this.attachments = this.attachments.filter((attachment) => !sameKey(home, attachment));
  }

  /** The references that the first message sends, without the home, which it sends apart. */
  refs(): AttachmentRef[] {
    return this.attachments
      .filter((attachment) => !isHome(this.home, attachment))
      .map(({ key, provider }) => ({ key, provider }));
  }

  /** Starts over with these attachments and this home, or with none. */
  reset(attachments: readonly Attachment[] = [], home: Attachment | null = null): void {
    const others = attachments.filter((attachment) => !isHome(home, attachment));
    this.attachments = home === null ? others : [home, ...others];
    this.home = home === null ? null : { key: home.key, provider: home.provider };
  }
}

/** The window's draft of a new conversation. */
export const conversationDraft = new ConversationDraftState();
