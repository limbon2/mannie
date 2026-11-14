// What the draft conversation of the new conversation's place gets when the
// chat view opens it next (ADR-0068; ADR-0046, decision 6; ADR-0051): the
// home item whose draft it is, and the attachments and the agent that a
// plugin's `startConversation`, a plan or an agent's page ask for. The view
// hands them to the core with the draft and lets go of them, and keeps the
// home, which names the draft's place.
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

/** What the draft of the new conversation's place gets when it opens next. */
export interface DraftIntent {
  /** The agent to choose, by its key, or `null` to keep the draft's own. */
  readonly agentKey: string | null;
  /** What to attach, without the home. */
  readonly attachments: AttachmentRef[];
}

/** What the new conversation's draft gets when it opens next. */
export class ConversationDraftState {
  /** The attachments, in the order they were given, the home first. */
  attachments: Attachment[] = $state([]);

  /** The home item whose draft opens, which stays attached, or `null` for a draft of the Chat. */
  home: AttachmentRef | null = $state(null);

  /** The agent to choose for the draft's replies, or `null` to keep its own (ADR-0059). */
  agentKey: string | null = $state(null);

  /**
   * The project whose draft the intent is for, `null` for the general
   * conversations' or the home's, or `undefined` for whichever draft shows
   * next. A draft of another place that still shows leaves the intent alone,
   * so it reaches the place that the window then shows.
   */
  projectId = $state<string | null>();

  /** Whether the draft gets something when it opens next. */
  get hasIntent(): boolean {
    const attaches = this.refs().length > 0;
    const choosesAgent = this.agentKey !== null;
    return attaches || choosesAgent;
  }

  /** Whether the draft of this place, which the home names when there is one, gets the intent. */
  isFor(projectId: string | null): boolean {
    const anyPlace = this.projectId === undefined;
    const thisPlace = this.projectId === projectId;
    return anyPlace || thisPlace;
  }

  /** Lets go of the home, so the next draft is one of the Chat's, and keeps the other attachments. */
  leaveHome(): void {
    const home = this.home;
    if (home === null) {
      return;
    }
    this.home = null;
    this.attachments = this.attachments.filter((attachment) => !sameKey(home, attachment));
  }

  /** The references of the attachments, without the home, which the draft's place names. */
  refs(): AttachmentRef[] {
    return this.attachments
      .filter((attachment) => !isHome(this.home, attachment))
      .map(({ key, provider }) => ({ key, provider }));
  }

  /** Hands over what the draft gets, and lets go of it; the home stays. */
  takeIntent(): DraftIntent {
    const intent = { agentKey: this.agentKey, attachments: this.refs() };
    // Nothing to let go of writes nothing, so a reader of `hasIntent`, such
    // as the chat view's effect that opens the draft, does not run again.
    if (!this.hasIntent) {
      return intent;
    }
    const home = this.home;
    this.attachments = this.attachments.filter((attachment) => isHome(home, attachment));
    this.agentKey = null;
    this.projectId = undefined;
    return intent;
  }

  /**
   * Starts over with these attachments and this home, or with none, for the
   * draft of the place that `projectId` names, or for whichever shows next.
   */
  reset(attachments: readonly Attachment[] = [], home: Attachment | null = null, projectId?: string | null): void {
    const others = attachments.filter((attachment) => !isHome(home, attachment));
    this.attachments = home === null ? others : [home, ...others];
    this.home = home === null ? null : { key: home.key, provider: home.provider };
    this.agentKey = null;
    this.projectId = home === null ? projectId : null;
  }
}

/** What the window's next new conversation gets. */
export const conversationDraft = new ConversationDraftState();
