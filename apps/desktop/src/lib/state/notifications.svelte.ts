// Notifications and tray badge state (docs/features/notifications.md;
// ADR-0074): tracks unread finished conversations pushed from the core, opens
// a conversation when the owner clicks a desktop notification, sends the
// localized labels from the catalog, and reports which conversation is open.
import { SvelteSet } from 'svelte/reactivity';

import { commands, type NotificationUpdate } from '#lib/generated/bindings.js';
import { batchChannel } from '#lib/utils/calls.js';
import { openConversation } from '#lib/utils/chat-urls.js';
import { notificationLabels } from '#lib/utils/notifications-messages.js';

/** Unread finished conversations and click-to-open navigation from the core. */
export class NotificationsState {
  #generation = 0;
  #lastViewed: string | null | undefined = undefined;
  #started = false;
  readonly #unread = new SvelteSet<string>();

  /** Conversations whose last completed or failed run has not been viewed yet. */
  get unreadIds(): ReadonlySet<string> {
    return this.#unread;
  }

  /** Whether `conversationId` currently has an unread finished run. */
  isUnread(conversationId: string): boolean {
    return this.#unread.has(conversationId);
  }

  /** Reports which conversation the main window currently displays. */
  reportViewed(conversationId: string | null): void {
    if (this.#lastViewed === conversationId) {
      return;
    }
    this.#lastViewed = conversationId;
    void commands.setViewedConversation({ conversationId });
  }

  /** Starts receiving unread updates and click navigation from the core. */
  start(): void {
    if (this.#started) {
      return;
    }
    this.#started = true;
    this.#lastViewed = undefined;
    void this.#subscribe();
  }

  /** Sends localized notification titles and the badged tray tooltip to the core. */
  syncLabels(): void {
    void commands.setNotificationLabels(notificationLabels());
  }

  #apply(update: NotificationUpdate): void {
    switch (update.type) {
      case 'unread_changed': {
        this.#unread.clear();
        for (const id of update.conversationIds) {
          this.#unread.add(id);
        }
        return;
      }
      case 'open_conversation': {
        void openConversation(update.conversationId);
      }
    }
  }

  async #subscribe(): Promise<void> {
    this.#generation += 1;
    const generation = this.#generation;
    const channel = batchChannel<NotificationUpdate>(
      (updates) => {
        if (generation !== this.#generation) {
          return;
        }
        for (const update of updates) {
          this.#apply(update);
        }
      },
      () => {
        // Resubscribing replays the current unread set and any pending click target.
        if (generation === this.#generation) {
          void this.#subscribe();
        }
      },
    );
    try {
      await commands.subscribeNotifications(channel);
    } catch {
      // A core that failed to open keeps no unread state for this page.
    }
  }
}

/** The window's notifications state. */
export const notifications = new NotificationsState();
