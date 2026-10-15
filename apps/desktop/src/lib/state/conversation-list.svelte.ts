// The conversation list (docs/features/chat/conversation-view.md, layout;
// projects.md): the whole list and the projects from the core first, then
// each conversation and project that is created, changed or deleted. The
// window follows it for as long as it runs. Updates arrive in batches, so the
// list announces each move of a conversation into a project as it applies
// it: a batch can hold a new conversation and its move.
import {
  type ChatError,
  commands,
  type ConversationListUpdate,
  type ConversationSummary,
  type ProjectView,
  type SubscriptionId,
} from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';
import { batchChannel, call } from '#lib/utils/calls.js';
import { groupConversations, type GroupedConversations } from '#lib/utils/project-groups.js';

/** The most recently changed first, as the core sorts them. */
function newestFirst(left: ConversationSummary, right: ConversationSummary): number {
  const byTime = right.updatedAt - left.updatedAt;
  if (byTime !== 0) {
    return byTime;
  }
  return right.id.localeCompare(left.id);
}

/** A general conversation that moved into a project. */
export interface ConversationMove {
  readonly conversationId: string;
  readonly projectId: string;
}

/** Whether a conversation's update moves it from the general conversations into a project. */
function moveOf(previous: ConversationSummary | undefined, next: ConversationSummary): ConversationMove | null {
  const wasGeneral = previous?.projectId === null;
  const projectId = next.projectId;
  const isInProject = projectId !== null;
  if (!wasGeneral || !isInProject) {
    return null;
  }
  return { conversationId: next.id, projectId };
}

/** The oldest first, as the core sorts them. */
function oldestFirst(left: ProjectView, right: ProjectView): number {
  const byTime = left.createdAt - right.createdAt;
  if (byTime !== 0) {
    return byTime;
  }
  return left.id.localeCompare(right.id);
}

/** The window's conversation list. */
export class ConversationList {
  conversations: ConversationSummary[] = $state([]);
  /** Why the list could not be read, if it could not. */
  failure: Failure<ChatError> | null = $state(null);
  /** The owner's projects, the oldest first. */
  projects: ProjectView[] = $state([]);
  status: 'error' | 'loading' | 'ready' = $state('loading');
  /** The conversations by project, as the sidebar shows them. */
  readonly grouped: GroupedConversations = $derived(groupConversations(this.conversations, this.projects));
  /** Counts subscriptions, so that the updates of an ended one change nothing. */
  #generation = 0;
  /** The functions that hear of moves; no state, since nothing renders them. */
  readonly #moveListeners: ((move: ConversationMove) => void)[] = [];
  #started = false;
  #subscription: SubscriptionId | null = null;

  /** The conversation with this ID, if the list has it. */
  find(id: string): ConversationSummary | undefined {
    return this.conversations.find((conversation) => conversation.id === id);
  }

  /** The project with this ID, if the list has it. */
  findProject(id: string): ProjectView | undefined {
    return this.projects.find((project) => project.id === id);
  }

  /**
   * Calls `listener` for each general conversation that moves into a
   * project, as the list applies the move. Returns a function that stops
   * the calls.
   */
  onMove(listener: (move: ConversationMove) => void): () => void {
    this.#moveListeners.push(listener);
    return () => {
      const index = this.#moveListeners.indexOf(listener);
      if (index !== -1) {
        this.#moveListeners.splice(index, 1);
      }
    };
  }

  /** Adds or replaces a project that a command returned, before its update arrives through the channel. */
  upsertProject(project: ProjectView): void {
    const others = this.projects.filter((existing) => existing.id !== project.id);
    this.projects = [...others, project].sort(oldestFirst);
  }

  /** Follows the list again after a failure. */
  retry(): void {
    void this.#subscribe();
  }

  /** Starts following the list. Later calls change nothing. */
  start(): void {
    if (this.#started) {
      return;
    }
    this.#started = true;
    void this.#subscribe();
  }

  #apply(update: ConversationListUpdate): void {
    switch (update.type) {
      case 'ended': {
        // A list that fell behind follows the list again; at shutdown the
        // window is about to close.
        if (update.reason === 'lagged') {
          void this.#subscribe();
        }
        return;
      }
      case 'project_removed': {
        this.projects = this.projects.filter((project) => project.id !== update.id);
        return;
      }
      case 'project_upserted': {
        this.upsertProject(update.project);
        return;
      }
      case 'removed': {
        this.conversations = this.conversations.filter((conversation) => conversation.id !== update.id);
        return;
      }
      case 'snapshot': {
        // A list that subscribes again after it fell behind can find moves
        // in the new snapshot.
        const previous: Partial<Record<string, ConversationSummary>> = Object.fromEntries(
          this.conversations.map((known) => [known.id, known]),
        );
        this.conversations = [...update.conversations].sort(newestFirst);
        this.projects = [...update.projects].sort(oldestFirst);
        this.status = 'ready';
        this.failure = null;
        for (const conversation of update.conversations) {
          this.#announce(moveOf(previous[conversation.id], conversation));
        }
        return;
      }
      case 'upserted': {
        const before = this.find(update.conversation.id);
        const others = this.conversations.filter((conversation) => conversation.id !== update.conversation.id);
        this.conversations = [...others, update.conversation].sort(newestFirst);
        this.#announce(moveOf(before, update.conversation));
      }
    }
  }

  #announce(move: ConversationMove | null): void {
    if (move === null) {
      return;
    }
    for (const listener of [...this.#moveListeners]) {
      listener(move);
    }
  }

  async #subscribe(): Promise<void> {
    this.#generation += 1;
    const generation = this.#generation;
    const previous = this.#subscription;
    this.#subscription = null;
    if (previous !== null) {
      void commands.unsubscribe({ subscriptionId: previous });
    }
    if (this.conversations.length === 0) {
      this.status = 'loading';
    }
    const channel = batchChannel<ConversationListUpdate>(
      (items) => {
        if (generation !== this.#generation) {
          return;
        }
        for (const item of items) {
          this.#apply(item);
        }
      },
      () => {
        if (generation === this.#generation) {
          void this.#subscribe();
        }
      },
    );
    const outcome = await call(() => commands.subscribeConversations(channel));
    if (generation !== this.#generation) {
      // A newer subscription replaced this one while it started.
      if (outcome.ok) {
        void commands.unsubscribe({ subscriptionId: outcome.data });
      }
      return;
    }
    if (outcome.ok) {
      this.#subscription = outcome.data;
      return;
    }
    this.failure = outcome.failure;
    this.status = 'error';
  }
}

/** The window's conversation list. */
export const conversationList = new ConversationList();
