import { beforeEach, describe, expect, it, vi } from 'vitest';

import { FileLinkState } from '../../../../../../apps/desktop/src/lib/state/file-link.svelte.js';

const core = vi.hoisted(() => ({ resolveFileLink: vi.fn(), openFileLink: vi.fn() }));
vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));
const link = { conversationId: 'containing-conversation', path: 'src/main.rs' };
const resolved = { target: 'file_manager', application: 'file_explorer', kind: 'file', path: 'C:/project/src/main.rs' };

beforeEach(() => {
  core.resolveFileLink.mockReset();
  core.openFileLink.mockReset();
});

describe('file-link actions', () => {
  it('resolves for a menu without launching and opens only the fixed target', async () => {
    const actions = new FileLinkState(link);
    expect(core.resolveFileLink).not.toHaveBeenCalled();
    expect(core.openFileLink).not.toHaveBeenCalled();
    core.resolveFileLink.mockResolvedValue({ status: 'ok', data: resolved });
    await actions.resolve();
    expect(core.resolveFileLink).toHaveBeenCalledWith(link);
    expect(actions.resolved).toEqual(resolved);
    expect(core.openFileLink).not.toHaveBeenCalled();
    core.openFileLink.mockResolvedValue({ status: 'ok', data: null });
    await actions.open();
    expect(core.openFileLink).toHaveBeenCalledWith({ link, target: 'file_manager' });
  });

  it('prevents duplicate activation and clears pending state after failure', async () => {
    const actions = new FileLinkState(link);
    let finish: ((value: unknown) => void) | undefined;
    core.openFileLink.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const pending = actions.open();
    expect(actions.pending).toBe('opening');
    await actions.open();
    expect(core.openFileLink).toHaveBeenCalledTimes(1);
    finish?.({ status: 'error', error: { code: 'open_failed' } });
    await pending;
    expect(actions.failure).toEqual({ code: 'open_failed' });
    expect(actions.pending).toBeNull();
    core.openFileLink.mockResolvedValue({ status: 'ok', data: null });
    await actions.open();
    expect(actions.failure).toBeNull();
  });

  it('reports resolution and transport failures without opening', async () => {
    const actions = new FileLinkState(link);
    core.resolveFileLink.mockResolvedValue({ status: 'error', error: { code: 'not_found' } });
    await actions.resolve();
    expect(actions.failure).toEqual({ code: 'not_found' });
    expect(actions.resolved).toBeNull();
    core.resolveFileLink.mockRejectedValue(new Error('disconnected'));
    await actions.resolve();
    expect(actions.failure).toEqual({ code: 'ipc' });
    expect(core.openFileLink).not.toHaveBeenCalled();
    core.openFileLink.mockRejectedValue(new Error('disconnected'));
    await actions.open();
    expect(actions.failure).toEqual({ code: 'ipc' });
    expect(actions.pending).toBeNull();
  });

  it('ignores an old response after the containing reference changes', async () => {
    const actions = new FileLinkState(link);
    let finish: ((value: unknown) => void) | undefined;
    core.resolveFileLink.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const pending = actions.resolve();
    actions.reset();
    finish?.({ status: 'ok', data: resolved });
    await pending;
    expect(actions.resolved).toBeNull();
    expect(actions.pending).toBeNull();
  });
});
