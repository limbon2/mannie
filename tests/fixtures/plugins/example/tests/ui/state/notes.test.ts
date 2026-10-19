// The plugin's notes: the list follows the plugin's channel while the
// plugin is on, a note that the command kept shows once even when the
// channel brings it too, a refused note keeps the reason, and the list
// forgets the notes as the plugin turns off.
import type * as Ipc from '@mannie/plugin-sdk/ipc';
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { Note } from '../../../ui/generated/bindings.js';
import { NoteList } from '../../../ui/state/notes.svelte.js';

const core = vi.hoisted(() => ({ addNote: vi.fn(), followNotes: vi.fn(), unsubscribe: vi.fn() }));

/** Hands batches of messages to the list, as the plugin's channel does. */
const channel = vi.hoisted(() => ({
  deliver: (_messages: readonly Ipc.ChannelMessage<Note>[]): void => undefined,
}));

vi.mock('../../../ui/generated/bindings.js', () => ({
  channels: { followNotes: core.followNotes },
  commands: { addNote: core.addNote },
}));
vi.mock('@mannie/plugin-sdk/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  batchChannel: (onItems: (messages: readonly Ipc.ChannelMessage<Note>[]) => void) => {
    channel.deliver = onItems;
    return {};
  },
  unsubscribe: core.unsubscribe,
}));

/** A list that follows the notes in a plugin context where the plugin is on until the test turns it off. */
function followedList(): { context: TestPluginContext; list: NoteList } {
  const context = new TestPluginContext('example', true);
  const list = new NoteList();
  list.follow(context);
  return { context, list };
}

beforeEach(() => {
  for (const command of Object.values(core)) {
    command.mockReset();
  }
  core.followNotes.mockResolvedValue({ data: 3, status: 'ok' });
});

describe('the notes', () => {
  it('are ready once the plugin follows them, and show each note that the channel brings', async () => {
    const { list } = followedList();
    const loading = list.status;
    await vi.waitFor(() => {
      expect(list.status).toBe('ready');
    });

    channel.deliver([{ item: { id: 'n1', text: 'First' }, kind: 'item' }]);

    expect(loading).toBe('loading');
    expect(list.notes).toEqual([{ id: 'n1', text: 'First' }]);
  });

  it('show a note that the command kept once, also when the channel brings it', async () => {
    core.addNote.mockResolvedValue({ data: { id: 'n2', text: 'Second' }, status: 'ok' });
    const { list } = followedList();
    await vi.waitFor(() => {
      expect(list.status).toBe('ready');
    });

    const kept = await list.add('Second');
    channel.deliver([{ item: { id: 'n2', text: 'Second' }, kind: 'item' }]);

    expect(kept).toBe(true);
    expect(core.addNote).toHaveBeenCalledWith({ text: 'Second' });
    expect(list.notes).toEqual([{ id: 'n2', text: 'Second' }]);
  });

  it('keep the reason when the host refused a note', async () => {
    core.addNote.mockResolvedValue({
      error: { code: 'invalid_input', description: '', field: 'text', origin: 'host' },
      status: 'error',
    });
    const list = new NoteList();

    const kept = await list.add(' ');

    expect(kept).toBe(false);
    expect(list.adding).toBe(false);
    expect(list.addFailure).toEqual({
      error: { code: 'invalid_input', description: '', field: 'text', origin: 'host' },
      kind: 'core',
    });
  });

  it('say why they could not be followed, and follow them again on request', async () => {
    core.followNotes.mockResolvedValueOnce({
      error: { code: 'plugin_off', description: '', origin: 'host', plugin: 'example' },
      status: 'error',
    });
    const { list } = followedList();
    await vi.waitFor(() => {
      expect(list.status).toBe('error');
    });

    list.retry();

    await vi.waitFor(() => {
      expect(list.status).toBe('ready');
    });
    expect(list.failure).toBeNull();
  });

  it('are forgotten as the plugin turns off, and followed again once it is on', async () => {
    const { context, list } = followedList();
    await vi.waitFor(() => {
      expect(list.status).toBe('ready');
    });
    channel.deliver([{ item: { id: 'n1', text: 'First' }, kind: 'item' }]);

    context.turnOff();
    const whileOff = [...list.notes];
    context.turnOn();

    expect(whileOff).toEqual([]);
    expect(core.unsubscribe).toHaveBeenCalledWith(3);
    await vi.waitFor(() => {
      expect(core.followNotes).toHaveBeenCalledTimes(2);
    });
  });
});
