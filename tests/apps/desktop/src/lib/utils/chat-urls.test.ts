// Where the chat view lives (docs/features/chat/conversation-view.md, layout;
// projects.md): the address of a conversation, and of a new conversation in a
// project or a general one.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  conversationQuery,
  newConversationProjectId,
  newConversationQuery,
  openConversationId,
} from '../../../../../../apps/desktop/src/lib/utils/chat-urls.js';

const page = vi.hoisted(() => ({ route: { id: '/' }, url: new URL('http://localhost/') }));

vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));
vi.mock('$app/state', () => ({ page }));

beforeEach(() => {
  page.route = { id: '/' };
  page.url = new URL('http://localhost/');
});

describe('chat URLs', () => {
  it('name a conversation, or the project of a new one', () => {
    expect(conversationQuery('c1')).toBe('conversation=c1&view=chat');
    expect(newConversationQuery()).toBe('view=chat');
    expect(newConversationQuery('p1')).toBe('project=p1&view=chat');
  });

  it('read the project only while the pane of a new conversation shows', () => {
    page.url = new URL('http://localhost/?project=p1&view=chat');
    expect(newConversationProjectId()).toBe('p1');
    expect(openConversationId()).toBeNull();

    page.url = new URL('http://localhost/?conversation=c1&project=p1&view=chat');
    expect(newConversationProjectId()).toBeNull();
    expect(openConversationId()).toBe('c1');

    page.url = new URL('http://localhost/?project=p1&view=other');
    expect(newConversationProjectId()).toBeNull();

    page.route = { id: '/settings' };
    page.url = new URL('http://localhost/settings?project=p1');
    expect(newConversationProjectId()).toBeNull();
  });
});
