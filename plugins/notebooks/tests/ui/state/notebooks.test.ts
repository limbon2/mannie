// The notebooks that the window follows: they are ready once the channel
// brought the first list, each list replaces the one before, a list that
// fell behind follows again, a failure can be tried again, a command's
// notebook shows until the channel brings it, and the list forgets the
// notebooks as the plugin turns off.
import type * as Ipc from '@mannie/plugin-sdk/ipc';
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { NotebookList, NotebookView, SourceView } from '../../../ui/generated/bindings.js';
import { NotebookLibrary } from '../../../ui/state/notebooks.svelte.js';

const core = vi.hoisted(() => ({ followNotebooks: vi.fn(), unsubscribe: vi.fn() }));

/** Hands batches of messages to the list, as the plugin's channel does. */
const channel = vi.hoisted(() => ({
  deliver: (_messages: readonly Ipc.ChannelMessage<NotebookList>[]): void => undefined,
}));

vi.mock('../../../ui/generated/bindings.js', () => ({ channels: { followNotebooks: core.followNotebooks } }));
vi.mock('@mannie/plugin-sdk/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  batchChannel: (onItems: (messages: readonly Ipc.ChannelMessage<NotebookList>[]) => void) => {
    channel.deliver = onItems;
    return {};
  },
  unsubscribe: core.unsubscribe,
}));

function source(id: string, state: SourceView['state']): SourceView {
  return {
    addedAt: 0,
    bytes: 10,
    embedded: 0,
    failure: null,
    id,
    included: true,
    indexedAt: null,
    kind: 'markdown',
    name: `${id}.md`,
    notebookId: 'notebook-1',
    pages: null,
    passages: 3,
    state,
  };
}

function notebook(id: string, sources: SourceView[] = []): NotebookView {
  return { createdAt: 0, id, name: `Notebook ${id}`, sources, updatedAt: 0 };
}

/** A list that follows the notebooks in a context where the plugin is on until the test turns it off. */
async function followedLibrary(): Promise<{ context: TestPluginContext; library: NotebookLibrary }> {
  const context = new TestPluginContext('notebooks', true);
  const library = new NotebookLibrary();
  library.follow(context);
  await vi.waitFor(() => {
    expect(core.followNotebooks).toHaveBeenCalled();
  });
  return { context, library };
}

beforeEach(() => {
  for (const command of Object.values(core)) {
    command.mockReset();
  }
  core.followNotebooks.mockResolvedValue({ data: 7, status: 'ok' });
});

describe('the notebooks', () => {
  it('are ready once the first list arrives, and each list replaces the one before', async () => {
    const { library } = await followedLibrary();
    await Promise.resolve();
    const beforeTheList = library.status;

    channel.deliver([{ item: { notebooks: [notebook('notebook-1')] }, kind: 'item' }]);
    const first = library.notebooks.map((known) => known.id);
    channel.deliver([{ item: { notebooks: [notebook('notebook-2'), notebook('notebook-1')] }, kind: 'item' }]);

    expect(beforeTheList).toBe('loading');
    expect(first).toEqual(['notebook-1']);
    expect(library.status).toBe('ready');
    expect(library.notebooks.map((known) => known.id)).toEqual(['notebook-2', 'notebook-1']);
  });

  it('follow again when the list fell behind', async () => {
    await followedLibrary();

    channel.deliver([{ kind: 'ended', reason: 'lagged' }]);

    await vi.waitFor(() => {
      expect(core.followNotebooks).toHaveBeenCalledTimes(2);
    });
    expect(core.unsubscribe).toHaveBeenCalledWith(7);
  });

  it('say why they could not be followed, and follow again on request', async () => {
    core.followNotebooks.mockResolvedValueOnce({
      error: { code: 'plugin_off', description: '', origin: 'host', plugin: 'notebooks' },
      status: 'error',
    });
    const { library } = await followedLibrary();
    await vi.waitFor(() => {
      expect(library.status).toBe('error');
    });

    library.retry();
    const whileFollowing = library.status;

    await vi.waitFor(() => {
      expect(library.failure).toBeNull();
    });
    expect(whileFollowing).toBe('loading');
    expect(core.followNotebooks).toHaveBeenCalledTimes(2);
  });

  it("show a command's notebook and source until the channel brings them", async () => {
    const { library } = await followedLibrary();
    channel.deliver([{ item: { notebooks: [notebook('notebook-1', [source('a', 'indexing')])] }, kind: 'item' }]);

    library.keep(notebook('notebook-2'));
    library.keep(notebook('notebook-1'));
    library.replaceSource(source('a', 'ready'));

    expect(library.notebooks.map((known) => known.id)).toEqual(['notebook-2', 'notebook-1']);
    expect(library.findSource('a')?.state).toBe('ready');
    expect(library.find('notebook-1')?.sources).toHaveLength(1);
  });

  it('are forgotten as the plugin turns off, and followed again once it is on', async () => {
    const { context, library } = await followedLibrary();
    channel.deliver([{ item: { notebooks: [notebook('notebook-1')] }, kind: 'item' }]);

    context.turnOff();
    const whileOff = { notebooks: [...library.notebooks], status: library.status };
    context.turnOn();

    expect(whileOff).toEqual({ notebooks: [], status: 'loading' });
    expect(core.unsubscribe).toHaveBeenCalledWith(7);
    await vi.waitFor(() => {
      expect(core.followNotebooks).toHaveBeenCalledTimes(2);
    });
  });
});
