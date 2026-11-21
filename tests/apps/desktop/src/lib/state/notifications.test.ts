// Notifications state (docs/features/notifications.md; ADR-0074): tracks unread
// finished conversations pushed from the core, opens a conversation when the
// owner clicks a desktop notification, reports the viewed conversation once per
// change, and resubscribes cleanly after channel gaps.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  NotificationLabels,
  NotificationUpdate,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { NotificationsState } from '../../../../../../apps/desktop/src/lib/state/notifications.svelte.js';
import type * as Calls from '../../../../../../apps/desktop/src/lib/utils/calls.js';
import { notificationLabels } from '../../../../../../apps/desktop/src/lib/utils/notifications-messages.js';

interface OkVoid {
  readonly data: null;
  readonly status: 'ok';
}

interface Subscriber {
  readonly onGap: () => void;
  readonly onItems: (items: readonly NotificationUpdate[]) => void;
}

const mocks = vi.hoisted(() => ({
  openConversation: vi.fn(),
  setNotificationLabels: vi.fn<(labels: NotificationLabels) => Promise<OkVoid>>(),
  setViewedConversation: vi.fn(),
  subscribeNotifications: vi.fn(),
  subscribers: [] as Subscriber[],
}));

vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({
  commands: {
    setNotificationLabels: mocks.setNotificationLabels,
    setViewedConversation: mocks.setViewedConversation,
    subscribeNotifications: mocks.subscribeNotifications,
  },
}));
vi.mock('../../../../../../apps/desktop/src/lib/utils/chat-urls.js', () => ({
  openConversation: mocks.openConversation,
}));
vi.mock('../../../../../../apps/desktop/src/lib/utils/calls.js', async (importOriginal) => {
  const original = await importOriginal<typeof Calls>();
  return {
    ...original,
    batchChannel: (onItems: Subscriber['onItems'], onGap: Subscriber['onGap']) => {
      mocks.subscribers.push({ onGap, onItems });
      return {};
    },
  };
});

function started(): NotificationsState {
  const state = new NotificationsState();
  state.start();
  return state;
}

function deliver(...updates: NotificationUpdate[]): void {
  mocks.subscribers.at(-1)?.onItems(updates);
}

beforeEach(() => {
  mocks.subscribers.length = 0;
  mocks.openConversation.mockReset().mockResolvedValue(undefined);
  mocks.setNotificationLabels.mockReset().mockResolvedValue({ data: null, status: 'ok' });
  mocks.setViewedConversation.mockReset().mockResolvedValue(undefined);
  mocks.subscribeNotifications.mockReset().mockResolvedValue(undefined);
});

describe('NotificationsState', () => {
  it('subscribes once and replaces the unread conversation set on updates', () => {
    const state = started();
    state.start();

    expect(mocks.subscribeNotifications).toHaveBeenCalledTimes(1);
    expect(state.isUnread('conv-1')).toBe(false);

    deliver({ conversationIds: ['conv-1', 'conv-2'], type: 'unread_changed' });
    expect(state.isUnread('conv-1')).toBe(true);
    expect(state.isUnread('conv-2')).toBe(true);
    expect(state.isUnread('conv-3')).toBe(false);

    deliver({ conversationIds: ['conv-2'], type: 'unread_changed' });
    expect(state.isUnread('conv-1')).toBe(false);
    expect(state.isUnread('conv-2')).toBe(true);
  });

  it('opens a conversation when the core sends a notification click target', () => {
    started();

    deliver({ conversationId: 'conv-42', type: 'open_conversation' });

    expect(mocks.openConversation).toHaveBeenCalledTimes(1);
    expect(mocks.openConversation).toHaveBeenCalledWith('conv-42');
  });

  it('reports the viewed conversation to the core only when it changes', () => {
    const state = started();

    state.reportViewed(null);
    state.reportViewed(null);
    state.reportViewed('conv-1');
    state.reportViewed('conv-1');
    state.reportViewed('conv-2');
    state.reportViewed(null);

    expect(mocks.setViewedConversation.mock.calls).toEqual([
      [{ conversationId: null }],
      [{ conversationId: 'conv-1' }],
      [{ conversationId: 'conv-2' }],
      [{ conversationId: null }],
    ]);
  });

  it('sends the catalog labels when synced', () => {
    const state = new NotificationsState();

    state.syncLabels();

    expect(mocks.setNotificationLabels).toHaveBeenCalledTimes(1);
    expect(mocks.setNotificationLabels).toHaveBeenCalledWith(notificationLabels());
  });

  it('subscribes again when batches are lost, and ignores the old subscription', () => {
    const state = started();

    mocks.subscribers[0]?.onGap();
    mocks.subscribers[0]?.onItems([{ conversationIds: ['stale'], type: 'unread_changed' }]);
    expect(state.isUnread('stale')).toBe(false);

    deliver({ conversationIds: ['fresh'], type: 'unread_changed' });
    expect(mocks.subscribeNotifications).toHaveBeenCalledTimes(2);
    expect(state.isUnread('fresh')).toBe(true);
  });
});
