// The notebooks in the sidebar and the view without a notebook (plugins/
// notebooks/docs/README.md, interface), rendered on the server side: a
// notebook's row lists "New chat" and the notebook's own chats, each as wide
// as the sidebar so that a long title is cut, and leaves out the chats that
// only attached the notebook (note 13); the view without a notebook leads to
// the latest one, or offers "New notebook" without any (note 14).
import type { ConversationSummary } from '@mannie/plugin-sdk/conversations';
import { render } from 'svelte/server';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import NotebookPanelRow from '../../../ui/components/NotebookPanelRow.svelte';
import NotebooksView from '../../../ui/components/NotebooksView.svelte';
import type { NotebookView } from '../../../ui/generated/bindings.js';
import { notebookLibrary } from '../../../ui/state/notebooks.svelte.js';
import { panelRows } from '../../../ui/state/panel-rows.svelte.js';

const conversations = vi.hoisted(() => ({ chats: [] as ConversationSummary[] }));
const parameters = vi.hoisted(() => ({ search: new URLSearchParams() }));

vi.mock('@mannie/plugin-sdk/conversations', () => ({
  conversationFailureMessage: () => '',
  deleteConversation: vi.fn(),
  detach: vi.fn(),
  followConversations: vi.fn(),
  listConversations: () => conversations.chats,
}));
vi.mock('@mannie/plugin-sdk/navigation', () => ({
  openConversation: vi.fn(),
  openSettings: vi.fn(),
  openView: vi.fn(),
  startConversation: vi.fn(),
  viewParameters: () => parameters.search,
}));

function notebook(id: string, name: string, updatedAt: number): NotebookView {
  return { createdAt: 0, id, name, sources: [], updatedAt };
}

function chat(id: string, title: string, home: string | null, attached: readonly string[]): ConversationSummary {
  return {
    attachments: attached.map((key) => ({ key, label: key, provider: 'notebooks' })),
    awaitingApproval: false,
    createdAt: 0,
    hasActiveRun: false,
    ...(home === null ? {} : { home: { key: home, provider: 'notebooks' } }),
    id,
    profileId: null,
    projectId: null,
    task: null,
    title,
    titleSource: 'owner',
    updatedAt: 0,
    workspace: null,
  };
}

beforeEach(() => {
  conversations.chats = [];
  parameters.search = new URLSearchParams();
  notebookLibrary.notebooks = [];
  notebookLibrary.status = 'ready';
  notebookLibrary.failure = null;
});

describe("a notebook's row in the sidebar", () => {
  it("lists New chat and the notebook's own chats, cut to the sidebar's width", () => {
    panelRows.set('notebook-1', true);
    conversations.chats = [
      chat('own-1', 'How does WALLIX handle SSH connections through its proxy?', 'notebook-1', ['notebook-1']),
      chat('mentioned', 'Named it with @', null, ['notebook-1']),
      chat('other', 'Another notebook', 'notebook-2', ['notebook-2']),
    ];

    const body = render(NotebookPanelRow, { props: { notebook: notebook('notebook-1', 'Guides', 1) } }).body;

    expect(body).toContain('data-notebooks-panel-new-chat');
    expect(body).toContain('data-notebooks-panel-chat="own-1"');
    expect(body).not.toContain('data-notebooks-panel-chat="mentioned"');
    expect(body).not.toContain('data-notebooks-panel-chat="other"');
    expect(body).not.toContain('data-notebooks-panel-source');
    // The class of each button of the list, which takes the sidebar's width.
    const classes = [...body.matchAll(/class="([^"]*)"[^<]*?data-notebooks-panel-(?:chat|new-chat)/gu)].map(
      (match) => match[1],
    );
    expect(classes).toHaveLength(2);
    for (const names of classes) {
      expect(names.split(' ')).toContain('w-full');
    }
  });
});

describe('the view without a notebook', () => {
  it('waits for the latest notebook, which it leads to', () => {
    notebookLibrary.notebooks = [notebook('old', 'Old', 1), notebook('new', 'New', 2)];

    const body = render(NotebooksView).body;

    expect(notebookLibrary.latest()?.id).toBe('new');
    expect(body).toContain('aria-busy="true"');
    expect(body).not.toContain('data-notebooks-list');
  });

  it('offers New notebook without any notebook', () => {
    const body = render(NotebooksView).body;

    expect(body).toContain('data-notebooks-none');
    expect(body).toContain('No notebooks yet');
    expect(body).toContain('New notebook');
  });

  it('says why the notebooks could not be loaded', () => {
    notebookLibrary.status = 'error';
    notebookLibrary.failure = { kind: 'ipc' };

    const body = render(NotebooksView).body;

    expect(body).toContain('The notebooks could not be loaded');
    expect(body).toContain('Try again');
  });
});
