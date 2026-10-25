// A notebook's page in its states (plugins/notebooks/docs/README.md, loading,
// empty and error states), rendered on the server side with the notebooks
// given: loading, a failure with "Try again", a notebook that is gone, a
// notebook without sources, sources in each state, the files that a drop
// refused, and only its sources, without its chats (note 16).
import type { ConversationSummary } from '@mannie/plugin-sdk/conversations';
import { render } from 'svelte/server';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import NotebookPage from '../../../ui/components/NotebookPage.svelte';
import type { NotebookView, SourceView } from '../../../ui/generated/bindings.js';
import { notebookActions } from '../../../ui/state/notebook-actions.svelte.js';
import { notebookLibrary } from '../../../ui/state/notebooks.svelte.js';

const conversations = vi.hoisted(() => ({
  chats: [] as ConversationSummary[],
  status: 'ready',
}));

vi.mock('@mannie/plugin-sdk/conversations', () => ({
  conversationFailureMessage: () => '',
  conversationsStatus: () => conversations.status,
  detach: vi.fn(),
  followConversations: vi.fn(),
  listConversations: () => conversations.chats,
  retryConversations: vi.fn(),
}));
vi.mock('@mannie/plugin-sdk/navigation', () => ({
  openConversation: vi.fn(),
  openSettings: vi.fn(),
  openSidePanel: vi.fn(),
  openView: vi.fn(),
  startConversation: vi.fn(),
  viewParameters: () => new URLSearchParams(),
}));

function source(id: string, fields: Partial<SourceView>): SourceView {
  return {
    addedAt: 0,
    bytes: 1_000,
    embedded: 3,
    failure: null,
    id,
    included: true,
    indexedAt: 1,
    kind: 'markdown',
    name: `${id}.md`,
    notebookId: 'notebook-1',
    pages: null,
    passages: 3,
    state: 'ready',
    ...fields,
  };
}

function notebook(sources: SourceView[]): NotebookView {
  return { createdAt: 0, id: 'notebook-1', name: 'Physics', sources, updatedAt: 0 };
}

function chat(id: string, title: string, notebookIds: readonly string[]): ConversationSummary {
  return {
    attachments: notebookIds.map((key) => ({ key, label: 'Physics', provider: 'notebooks' })),
    awaitingApproval: false,
    createdAt: 0,
    hasActiveRun: false,
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

function shown(): string {
  return render(NotebookPage, { props: { notebookId: 'notebook-1' } }).body;
}

beforeEach(() => {
  notebookLibrary.status = 'ready';
  notebookLibrary.failure = null;
  notebookLibrary.notebooks = [];
  notebookActions.refusals = {};
  notebookActions.addFailures = {};
  notebookActions.adding = {};
  conversations.chats = [];
  conversations.status = 'ready';
});

describe('NotebookPage', () => {
  it('shows that it loads the notebook', () => {
    notebookLibrary.status = 'loading';

    const body = shown();

    expect(body).toContain('aria-busy="true"');
    expect(body).toContain('Loading the notebook…');
  });

  it('says why the notebook could not be loaded, with "Try again"', () => {
    notebookLibrary.status = 'error';
    notebookLibrary.failure = { kind: 'ipc' };

    const body = shown();

    expect(body).toContain('The notebook could not be loaded');
    expect(body).toContain('Try again');
  });

  it('says that a notebook is gone, with the way to the latest one', () => {
    const body = shown();

    expect(body).toContain('This notebook no longer exists');
    expect(body).toContain('Open the latest notebook');
  });

  it('offers to add sources to a notebook without any', () => {
    notebookLibrary.notebooks = [notebook([])];

    const body = shown();

    expect(body).toContain('No sources yet');
    expect(body).toContain('Add sources…');
    expect(body).toContain('Paste text…');
    expect(body).toContain('New chat');
  });

  it('shows each source in its state, with why it failed or waits', () => {
    notebookLibrary.notebooks = [
      notebook([
        source('ready', {}),
        source('paused', { embedded: 1, failure: 'server_unreachable', state: 'indexing' }),
        source('failed', { failure: 'server_error', state: 'failed' }),
        source('waiting', { embedded: 0, state: 'waiting' }),
      ]),
    ];

    const body = shown();

    expect(body).toContain('data-notebooks-state="ready"');
    expect(body).toContain('3 passages');
    expect(body).toContain('data-notebooks-state="paused"');
    expect(body).toContain('1 of 3 passages');
    expect(body).toContain('The embedding server could not be reached. Mannie tries again in a few minutes.');
    expect(body).toContain('data-notebooks-retry');
    expect(body).toContain('The embedding server failed.');
    expect(body).toContain('Choose an embedding model');
    expect(body).toContain('Use in answers');
  });

  it('lists the files that were not added, with why', () => {
    notebookLibrary.notebooks = [notebook([source('ready', {})])];
    notebookActions.refusals = { 'notebook-1': [{ name: 'photo.png', reason: 'unsupported_type' }] };

    const body = shown();

    expect(body).toContain('A file was not added');
    expect(body).toContain('photo.png');
    expect(body).toContain('Only PDF, Markdown and text files can be added.');
  });

  it('shows only its sources, since the sidebar lists the notebooks and their chats', () => {
    notebookLibrary.notebooks = [notebook([source('ready', {})])];
    conversations.chats = [chat('chat-1', 'Lenses', ['notebook-1'])];

    const body = shown();

    expect(body).toContain('data-notebooks-sources');
    expect(body).toContain('data-notebooks-new-chat');
    expect(body).not.toContain('Lenses');
    expect(body).not.toContain('All notebooks');
  });
});
