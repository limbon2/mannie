// What the owner does with notebooks and sources (plugins/notebooks/docs/
// README.md, notebooks and sources): a new notebook opens its page, a taken
// name keeps the dialog open with the reason, a drop uploads only the files
// that the plugin reads, in parts, and gathers the refusals, a switch shows
// its new value while its call runs, deleting a notebook deletes its own
// chats first, takes it from the other chats that use it and says when one
// stayed, and "New chat" starts a chat that belongs to the notebook.
import type { ConversationSummary } from '@mannie/plugin-sdk/conversations';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { NotebookView, SourceView } from '../../../ui/generated/bindings.js';
import { UPLOAD_PART_BYTES } from '../../../ui/generated/plugin.js';
import { NotebookActions } from '../../../ui/state/notebook-actions.svelte.js';
import { notebookLibrary } from '../../../ui/state/notebooks.svelte.js';

const core = vi.hoisted(() => ({
  addTextSource: vi.fn(),
  addUploadPart: vi.fn(),
  cancelUpload: vi.fn(),
  createNotebook: vi.fn(),
  deleteNotebook: vi.fn(),
  finishUpload: vi.fn(),
  pickSources: vi.fn(),
  removeSource: vi.fn(),
  renameNotebook: vi.fn(),
  retrySource: vi.fn(),
  setSourceIncluded: vi.fn(),
  startUpload: vi.fn(),
}));

const conversations = vi.hoisted(() => ({
  conversationFailureMessage: vi.fn(() => 'The window could not reach the core.'),
  deleteConversation: vi.fn(),
  detach: vi.fn(),
  listConversations: vi.fn((): ConversationSummary[] => []),
}));

const navigation = vi.hoisted(() => ({
  openView: vi.fn(),
  startConversation: vi.fn(),
  viewParameters: vi.fn(() => new URLSearchParams()),
}));

vi.mock('../../../ui/generated/bindings.js', () => ({ commands: core }));
vi.mock('@mannie/plugin-sdk/conversations', () => conversations);
vi.mock('@mannie/plugin-sdk/navigation', () => navigation);

const NOTEBOOK: NotebookView = { createdAt: 0, id: 'notebook-1', name: 'Physics', sources: [], updatedAt: 0 };

const SOURCE: SourceView = {
  addedAt: 0,
  bytes: 10,
  embedded: 3,
  failure: null,
  id: 'source-1',
  included: true,
  indexedAt: 1,
  kind: 'markdown',
  name: 'notes.md',
  notebookId: 'notebook-1',
  pages: null,
  passages: 3,
  state: 'ready',
};

function chat(id: string, notebookIds: readonly string[]): ConversationSummary {
  return {
    attachments: notebookIds.map((key) => ({ key, label: key, provider: 'notebooks' })),
    awaitingApproval: false,
    createdAt: 0,
    hasActiveRun: false,
    id,
    profileId: null,
    projectId: null,
    task: null,
    title: id,
    titleSource: 'owner',
    updatedAt: 0,
    workspace: null,
  };
}

/** A chat that belongs to a notebook (ADR-0051). */
function ownChat(id: string, notebookId: string): ConversationSummary {
  return { ...chat(id, [notebookId]), home: { key: notebookId, provider: 'notebooks' } };
}

function pluginError(code: string, fields: Record<string, unknown> = {}): unknown {
  return { error: { code, description: '', origin: 'plugin', ...fields }, status: 'error' };
}

/** The value, which the test expects to be there. */
function present<T>(value: T | null): T {
  expect(value).not.toBeNull();
  return value as T;
}

beforeEach(() => {
  vi.resetAllMocks();
  conversations.conversationFailureMessage.mockReturnValue('The window could not reach the core.');
  conversations.listConversations.mockReturnValue([]);
  navigation.viewParameters.mockReturnValue(new URLSearchParams());
  notebookLibrary.notebooks = [];
});

describe('the actions on notebooks', () => {
  it('create a notebook with its name trimmed, and open its page', async () => {
    core.createNotebook.mockResolvedValue({ data: NOTEBOOK, status: 'ok' });
    const actions = new NotebookActions();
    actions.startCreate();
    const request = present(actions.name);
    request.name = '  Physics ';

    await actions.confirmName(request);

    expect(core.createNotebook).toHaveBeenCalledWith({ name: 'Physics' });
    expect(actions.nameOpen).toBe(false);
    expect(notebookLibrary.find('notebook-1')).toEqual(NOTEBOOK);
    expect(navigation.openView).toHaveBeenCalledWith('notebooks', { notebook: 'notebook-1' }, undefined);
  });

  it('keep the name dialog open with the reason when another notebook has the name', async () => {
    core.renameNotebook.mockResolvedValue(pluginError('name_taken'));
    const actions = new NotebookActions();
    actions.startRename(NOTEBOOK);
    const request = present(actions.name);
    request.name = 'Optics';

    await actions.confirmName(request);

    expect(core.renameNotebook).toHaveBeenCalledWith({ name: 'Optics', notebookId: 'notebook-1' });
    expect(actions.nameOpen).toBe(true);
    expect(request.failure).toEqual({
      error: { code: 'name_taken', description: '', origin: 'plugin' },
      kind: 'core',
    });
    expect(navigation.openView).not.toHaveBeenCalled();
  });

  it('upload only the dropped files that the plugin reads, and gather the refusals', async () => {
    core.startUpload.mockImplementation(({ name }: { name: string }) =>
      Promise.resolve({ data: { uploadId: `upload-${name}` }, status: 'ok' }),
    );
    core.addUploadPart.mockResolvedValue({ data: null, status: 'ok' });
    core.finishUpload.mockImplementation(({ uploadId }: { uploadId: string }) =>
      Promise.resolve(
        uploadId === 'upload-copy.md'
          ? pluginError('refused', { reason: 'duplicate' })
          : { data: SOURCE, status: 'ok' },
      ),
    );
    const actions = new NotebookActions();

    await actions.drop(NOTEBOOK, [
      new File(['# Optics'], 'notes.md'),
      new File(['x'], 'photo.png'),
      new File(['# Optics'], 'copy.md'),
    ]);

    expect(core.startUpload).toHaveBeenCalledTimes(2);
    expect(core.startUpload).toHaveBeenCalledWith({ name: 'notes.md', notebookId: 'notebook-1' });
    expect(core.addUploadPart).toHaveBeenCalledWith({
      data: btoa('# Optics'),
      index: 0,
      uploadId: 'upload-notes.md',
    });
    expect(core.finishUpload).toHaveBeenCalledWith({ uploadId: 'upload-notes.md' });
    expect(actions.refusals['notebook-1']).toEqual([
      { name: 'photo.png', reason: 'unsupported_type' },
      { name: 'copy.md', reason: 'duplicate' },
    ]);
    expect(actions.adding['notebook-1']).toBeUndefined();
  });

  it('send a large file in parts, and cancel an upload whose part failed', async () => {
    core.startUpload.mockResolvedValue({ data: { uploadId: 'upload-1' }, status: 'ok' });
    core.addUploadPart
      .mockResolvedValueOnce({ data: null, status: 'ok' })
      .mockResolvedValueOnce(pluginError('upload_not_found'));
    core.cancelUpload.mockResolvedValue({ data: null, status: 'ok' });
    const large = new File([new Uint8Array(UPLOAD_PART_BYTES + 1)], 'guide.pdf');
    const actions = new NotebookActions();

    await actions.drop(NOTEBOOK, [large]);

    expect(core.addUploadPart).toHaveBeenCalledTimes(2);
    expect(core.addUploadPart).toHaveBeenLastCalledWith({ data: btoa('\0'), index: 1, uploadId: 'upload-1' });
    expect(core.cancelUpload).toHaveBeenCalledWith({ uploadId: 'upload-1' });
    expect(core.finishUpload).not.toHaveBeenCalled();
    expect(actions.refusals['notebook-1']).toEqual([
      {
        name: 'guide.pdf',
        reason: { error: { code: 'upload_not_found', description: '', origin: 'plugin' }, kind: 'core' },
      },
    ]);
  });

  it("keep the files that the dialog's choice refused, and a dialog that failed", async () => {
    core.pickSources
      .mockResolvedValueOnce({
        data: { added: [SOURCE], refused: [{ name: 'a.md', reason: 'duplicate' }] },
        status: 'ok',
      })
      .mockResolvedValueOnce(pluginError('dialog_failed'));
    const actions = new NotebookActions();

    await actions.pick(NOTEBOOK);
    const refused = actions.refusals['notebook-1'];
    await actions.pick(NOTEBOOK);
    actions.dismissRefusals('notebook-1');

    expect(core.pickSources).toHaveBeenCalledWith({ filterName: 'Documents', notebookId: 'notebook-1' });
    expect(refused).toEqual([{ name: 'a.md', reason: 'duplicate' }]);
    expect(actions.addFailures['notebook-1']).toEqual({
      error: { code: 'dialog_failed', description: '', origin: 'plugin' },
      kind: 'core',
    });
    expect(actions.refusals['notebook-1']).toBeUndefined();
    expect(actions.picking).toBeNull();
  });

  it("show a switch's new value while its call runs, and the stored one after a failure", async () => {
    let answer: (result: unknown) => void = () => undefined;
    core.setSourceIncluded.mockReturnValue(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    const actions = new NotebookActions();

    const switching = actions.setIncluded(SOURCE, false);
    const whileRunning = actions.isIncluded(SOURCE);
    answer(pluginError('storage_failed'));
    await switching;

    expect(core.setSourceIncluded).toHaveBeenCalledWith({ included: false, sourceId: 'source-1' });
    expect(whileRunning).toBe(false);
    expect(actions.isIncluded(SOURCE)).toBe(true);
    expect(actions.sourceFailures['source-1']).toBeDefined();
  });

  it('delete a notebook with its own chats first, and take it from the other chats that use it', async () => {
    conversations.listConversations.mockReturnValue([
      chat('chat-1', ['notebook-1']),
      chat('chat-2', ['other']),
      ownChat('own-1', 'notebook-1'),
      ownChat('own-2', 'other'),
    ]);
    conversations.deleteConversation.mockResolvedValue({ data: null, ok: true });
    conversations.detach.mockResolvedValue({ data: null, ok: true });
    core.deleteNotebook.mockResolvedValue({ data: null, status: 'ok' });
    const actions = new NotebookActions();
    actions.startDelete(NOTEBOOK);
    const request = present(actions.deleting);

    await actions.confirmDelete(request);

    expect([request.ownChats, request.usingChats]).toEqual([1, 1]);
    expect(conversations.deleteConversation.mock.calls).toEqual([['own-1']]);
    expect(conversations.deleteConversation.mock.invocationCallOrder[0]).toBeLessThan(
      core.deleteNotebook.mock.invocationCallOrder[0] ?? 0,
    );
    expect(core.deleteNotebook).toHaveBeenCalledWith({ notebookId: 'notebook-1' });
    expect(conversations.detach.mock.calls).toEqual([['chat-1', { key: 'notebook-1', provider: 'notebooks' }]]);
    expect(actions.deleteOpen).toBe(false);
  });

  it('keep a notebook whose own chat was not deleted, and say why', async () => {
    conversations.listConversations.mockReturnValue([ownChat('own-1', 'notebook-1'), ownChat('own-2', 'notebook-1')]);
    conversations.deleteConversation
      .mockResolvedValueOnce({ data: null, ok: true })
      .mockResolvedValueOnce({ failure: { kind: 'ipc' }, ok: false });
    const actions = new NotebookActions();
    actions.startDelete(NOTEBOOK);
    const request = present(actions.deleting);

    await actions.confirmDelete(request);

    expect(request.chatFailure).toBe('The window could not reach the core.');
    expect(request.deleting).toBe(false);
    expect(core.deleteNotebook).not.toHaveBeenCalled();
    expect(actions.deleteOpen).toBe(true);
  });

  it('say when a chat kept the deleted notebook, and leave the page of a deleted notebook', async () => {
    conversations.listConversations.mockReturnValue([chat('chat-1', ['notebook-1'])]);
    conversations.detach.mockResolvedValue({ failure: { kind: 'ipc' }, ok: false });
    core.deleteNotebook.mockResolvedValue(pluginError('notebook_not_found'));
    navigation.viewParameters.mockReturnValue(new URLSearchParams({ notebook: 'notebook-1' }));
    const actions = new NotebookActions();
    actions.startDelete(NOTEBOOK);
    const request = present(actions.deleting);

    await actions.confirmDelete(request);

    expect(request.failure).toBeNull();
    expect(request.detachFailure).toBe('The window could not reach the core.');
    expect(actions.deleteOpen).toBe(true);
    expect(navigation.openView).toHaveBeenCalledWith('notebooks');
  });

  it('start a chat that belongs to the notebook', async () => {
    const actions = new NotebookActions();

    await actions.newChat(NOTEBOOK);

    expect(navigation.startConversation).toHaveBeenCalledWith({
      home: { key: 'notebook-1', label: 'Physics', provider: 'notebooks' },
    });
  });
});
