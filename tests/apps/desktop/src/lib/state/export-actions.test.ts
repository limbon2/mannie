import { beforeEach, describe, expect, it, vi } from 'vitest';

import { ExportActions } from '../../../../../../apps/desktop/src/lib/state/export-actions.svelte.js';

const core = vi.hoisted(() => ({ exportConversation: vi.fn() }));
vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));

beforeEach(() => {
  core.exportConversation.mockReset();
});

describe('conversation export', () => {
  it('treats a canceled native picker as no action', async () => {
    const actions = new ExportActions();
    core.exportConversation.mockResolvedValue({ status: 'ok', data: { kind: 'canceled' } });
    await actions.start('conversation');
    expect(actions.open).toBe(false);
    expect(actions.result).toBeNull();
    expect(actions.busy).toBe(false);
  });

  it('preserves the saved folder and omissions for result feedback', async () => {
    const actions = new ExportActions();
    const saved = {
      kind: 'saved',
      destination: 'D:\\Exports\\conversation',
      warnings: [{ conversationId: 'child', reason: { code: 'host_unavailable' }, reference: 'child' }],
    };
    core.exportConversation.mockResolvedValue({ status: 'ok', data: saved });
    await actions.start('conversation');
    expect(core.exportConversation).toHaveBeenCalledWith({ conversationId: 'conversation' });
    expect(actions.result).toEqual(saved);
    expect(actions.open).toBe(true);
  });

  it('keeps typed and transport failures actionable and permits another attempt', async () => {
    const actions = new ExportActions();
    core.exportConversation.mockResolvedValueOnce({ status: 'error', error: { code: 'write_failed' } });
    await actions.start('conversation');
    expect(actions.failure).toEqual({ kind: 'core', error: { code: 'write_failed' } });
    expect(actions.open).toBe(true);
    core.exportConversation.mockRejectedValueOnce(new Error('disconnected'));
    await actions.start('conversation');
    expect(actions.failure).toEqual({ kind: 'ipc' });
    expect(actions.busy).toBe(false);
    core.exportConversation.mockResolvedValueOnce({
      status: 'ok',
      data: { kind: 'saved', destination: 'saved', warnings: [] },
    });
    await actions.start('conversation');
    expect(actions.failure).toBeNull();
    expect(actions.result?.destination).toBe('saved');
  });

  it('prevents a duplicate request while the native picker or export is active', async () => {
    const actions = new ExportActions();
    let finish: ((value: unknown) => void) | undefined;
    core.exportConversation.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
    const pending = actions.start('first');
    await actions.start('second');
    expect(actions.conversationId).toBe('first');
    expect(core.exportConversation).toHaveBeenCalledTimes(1);
    finish?.({ status: 'ok', data: { kind: 'canceled' } });
    await pending;
    expect(actions.busy).toBe(false);
  });
});
