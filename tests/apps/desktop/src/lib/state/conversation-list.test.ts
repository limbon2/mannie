// Moves in the conversation list (docs/features/chat/projects.md, the
// sidebar): the list announces each move of a general conversation into a
// project as it applies it, also when one batch of updates adds the
// conversation and moves it, and never for a conversation that starts in a
// project.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  ConversationListUpdate,
  ConversationSummary,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  ConversationList,
  type ConversationMove,
} from '../../../../../../apps/desktop/src/lib/state/conversation-list.svelte.js';
import type * as Calls from '../../../../../../apps/desktop/src/lib/utils/calls.js';

const core = vi.hoisted(() => ({ subscribeConversations: vi.fn(), unsubscribe: vi.fn() }));

/** Hands batches of updates to the list, as the core's channel does. */
const channel = vi.hoisted(() => ({ deliver: (_updates: readonly ConversationListUpdate[]): void => undefined }));

vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));
vi.mock('../../../../../../apps/desktop/src/lib/utils/calls.js', async (importOriginal) => ({
  ...(await importOriginal<typeof Calls>()),
  batchChannel: (onItems: (updates: readonly ConversationListUpdate[]) => void) => {
    channel.deliver = onItems;
    return {};
  },
}));

const PROJECT_ID = '0199e0f2-0000-7000-a000-000000000001';
const SNAPSHOT: ConversationListUpdate = { conversations: [], projects: [], type: 'snapshot' };

function summary(projectId: string | null, title = 'A conversation'): ConversationSummary {
  return {
    awaitingApproval: false,
    createdAt: 1,
    hasActiveRun: false,
    id: '0199e0f2-0000-7000-8000-000000000001',
    profileId: null,
    projectId,
    task: null,
    title,
    titleSource: 'first_message',
    updatedAt: 1,
    workspace: null,
  };
}

function upserted(conversation: ConversationSummary): ConversationListUpdate {
  return { conversation, type: 'upserted' };
}

/** A list that follows the core's updates, with the moves it announced. */
function followedList(): { list: ConversationList; moves: ConversationMove[] } {
  core.subscribeConversations.mockResolvedValue({ data: 'subscription', status: 'ok' });
  const list = new ConversationList();
  list.start();
  const moves: ConversationMove[] = [];
  list.onMove((move) => moves.push(move));
  return { list, moves };
}

beforeEach(() => {
  core.subscribeConversations.mockReset();
  core.unsubscribe.mockReset();
});

describe('moves in the conversation list', () => {
  it('announces a move that comes in one batch with the new conversation', () => {
    const { list, moves } = followedList();
    channel.deliver([SNAPSHOT]);

    channel.deliver([upserted(summary(null)), upserted(summary(PROJECT_ID))]);

    expect(moves).toEqual([{ conversationId: summary(null).id, projectId: PROJECT_ID }]);
    expect(list.find(summary(null).id)?.projectId).toBe(PROJECT_ID);
  });

  it('announces no move for a conversation that starts in a project or changes there', () => {
    const { moves } = followedList();
    channel.deliver([SNAPSHOT]);

    channel.deliver([upserted(summary(PROJECT_ID))]);
    channel.deliver([upserted(summary(PROJECT_ID, 'Renamed'))]);

    expect(moves).toEqual([]);
  });

  it('announces a move that a new snapshot shows', () => {
    const { moves } = followedList();
    channel.deliver([{ ...SNAPSHOT, conversations: [summary(null)] }]);

    channel.deliver([{ ...SNAPSHOT, conversations: [summary(PROJECT_ID)] }]);

    expect(moves).toEqual([{ conversationId: summary(null).id, projectId: PROJECT_ID }]);
  });

  it('stops announcing moves to a listener that stopped', () => {
    const { list } = followedList();
    const heard: ConversationMove[] = [];
    const stop = list.onMove((move) => heard.push(move));
    channel.deliver([SNAPSHOT, upserted(summary(null))]);

    stop();
    channel.deliver([upserted(summary(PROJECT_ID))]);

    expect(heard).toEqual([]);
  });
});

describe("the Chat's conversations", () => {
  it('leave out the conversations that belong to a plugin’s item, which the plugin lists (ADR-0051)', () => {
    const { list } = followedList();
    const general = { ...summary(null, 'General'), id: '0199e0f2-0000-7000-8000-000000000002' };
    const notebooksChat = { ...summary(null, 'A notebook’s chat'), home: { key: 'n1', provider: 'notebooks' } };

    channel.deliver([{ ...SNAPSHOT, conversations: [general, notebooksChat] }]);

    expect(list.conversations).toHaveLength(2);
    expect(list.chats.map((conversation) => conversation.title)).toEqual(['General']);
    expect(list.grouped.general.map((conversation) => conversation.title)).toEqual(['General']);
  });
});
