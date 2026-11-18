import type * as Ipc from '@mannie/plugin-sdk/ipc';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { GraphicsEvent } from '../../../ui/generated/bindings.js';
import { ProjectViewport } from '../../../ui/state/viewport.svelte.js';

type Deliver = (items: readonly Ipc.ChannelMessage<GraphicsEvent>[]) => void;
const core = vi.hoisted(() => ({
  deliver: [] as Deliver[],
  followViewport: vi.fn(),
  placeViewport: vi.fn(),
  setViewportColor: vi.fn(),
  unsubscribe: vi.fn(),
}));

vi.mock('../../../ui/generated/bindings.js', () => ({
  channels: { followViewport: core.followViewport },
  commands: { placeViewport: core.placeViewport, setViewportColor: core.setViewportColor },
}));
vi.mock('@mannie/plugin-sdk/ipc', async (original) => ({
  ...(await original<typeof Ipc>()),
  batchChannel: (onItems: Deliver) => {
    core.deliver.push(onItems);
    return {};
  },
  unsubscribe: core.unsubscribe,
}));

const rect = {
  height: 60,
  width: 120,
  x: 10,
  y: 20,
};
const shown = {
  bounds: rect,
  clip: rect,
  scaleFactor: 1.25,
  visible: true,
};
const ok = { data: null, status: 'ok' };

function ready(lease: string, channel = 0): void {
  core.deliver[channel]?.([{ item: { kind: 'ready', lease }, kind: 'item' }]);
}

beforeEach(() => {
  core.deliver.length = 0;
  vi.resetAllMocks();
  core.followViewport.mockResolvedValue({ data: 2, status: 'ok' });
  core.setViewportColor.mockResolvedValue(ok);
  core.placeViewport.mockResolvedValue(ok);
});

describe('the Designer viewport', () => {
  it('restores the project color before showing the native scene and keeps projects separate', async () => {
    const first = new ProjectViewport('color');
    expect(first.hue).toBe(0);
    first.changeHue(240);
    const session = new ProjectViewport('color');
    expect(new ProjectViewport('other').hue).toBe(0);
    session.place(shown);
    await session.open();
    ready('first');

    await vi.waitFor(() => expect(session.loading).toBe(false));
    expect(core.setViewportColor).toHaveBeenCalledWith({ hue: 240, lease: 'first', projectId: 'color' });
    expect(core.placeViewport).toHaveBeenCalledWith({ lease: 'first', placement: shown, projectId: 'color' });
    expect(core.setViewportColor.mock.invocationCallOrder[0]).toBeLessThan(
      core.placeViewport.mock.invocationCallOrder[0],
    );
  });

  it('sends the latest hue during a drag, prioritizes a queued hide and releases on close', async () => {
    const paused = Promise.withResolvers<typeof ok>();
    core.setViewportColor.mockReturnValueOnce(paused.promise);
    const session = new ProjectViewport('drag');
    await session.open();
    session.place(shown);
    ready('dragging');
    session.changeHue(120);
    session.changeHue(240);
    const hidden = { ...shown, visible: false };
    session.place(hidden);
    paused.resolve(ok);

    await vi.waitFor(() => expect(session.loading).toBe(false));
    expect(core.setViewportColor).toHaveBeenCalledTimes(2);
    expect(core.setViewportColor).toHaveBeenNthCalledWith(2, { hue: 240, lease: 'dragging', projectId: 'drag' });
    expect(core.placeViewport).toHaveBeenCalledExactlyOnceWith({
      lease: 'dragging',
      placement: hidden,
      projectId: 'drag',
    });
    expect(core.placeViewport.mock.invocationCallOrder[0]).toBeLessThan(
      core.setViewportColor.mock.invocationCallOrder[1],
    );
    session.close();
    expect(core.unsubscribe).toHaveBeenCalledWith(2);
  });

  it('releases a late opening before a new project opens and skips views already unmounted', async () => {
    const pending = Promise.withResolvers<{ data: number; status: string }>();
    const released = Promise.withResolvers<undefined>();
    core.followViewport.mockReturnValueOnce(pending.promise);
    core.unsubscribe.mockReturnValueOnce(released.promise);
    const old = new ProjectViewport('old');
    const opening = old.open();
    await vi.waitFor(() => expect(core.followViewport).toHaveBeenCalledTimes(1));
    old.close();
    const skipped = new ProjectViewport('between');
    void skipped.open();
    skipped.close();
    const current = new ProjectViewport('new');
    const nextOpening = current.open();
    ready('expired');
    pending.resolve({ data: 7, status: 'ok' });
    await vi.waitFor(() => expect(core.unsubscribe).toHaveBeenCalledWith(7));
    expect(core.followViewport).toHaveBeenCalledTimes(1);
    released.resolve(undefined);
    await Promise.all([opening, nextOpening]);
    expect(core.followViewport).toHaveBeenCalledTimes(2);
    expect(core.followViewport).toHaveBeenLastCalledWith({ projectId: 'new' }, expect.anything());
    ready('current', 1);
    await vi.waitFor(() => expect(current.loading).toBe(false));
    expect(core.setViewportColor).toHaveBeenCalledExactlyOnceWith({ hue: 0, lease: 'current', projectId: 'new' });
  });

  it('retries a render failure on a new lease and ignores late events on the previous channel', async () => {
    const session = new ProjectViewport('retry');
    await session.open();
    ready('failed');
    core.deliver[0]?.([{ item: { error: { code: 'render_failed' }, kind: 'failed' }, kind: 'item' }]);
    expect(session.failure?.retry).toBe(true);
    await session.open();
    ready('obsolete');
    ready('new', 1);
    await vi.waitFor(() => expect(session.loading).toBe(false));
    expect(session.failure).toBeNull();
    expect(core.setViewportColor).toHaveBeenCalledTimes(2);
    expect(core.setViewportColor).toHaveBeenLastCalledWith({ hue: 0, lease: 'new', projectId: 'retry' });
  });

  it('shows unsupported platforms without retrying an unavailable backend', async () => {
    core.followViewport.mockResolvedValue({
      error: { code: 'unsupported_platform', origin: 'plugin' },
      status: 'error',
    });
    const session = new ProjectViewport('linux');
    await session.open();
    expect(session.failure?.retry).toBe(false);
    expect(session.loading).toBe(false);
    expect(core.placeViewport).not.toHaveBeenCalled();
  });

  it('reports a host failure on retry and can open successfully afterward', async () => {
    const session = new ProjectViewport('recover');
    core.followViewport.mockRejectedValueOnce(new Error('disconnected'));
    await session.open();
    expect(session.failure?.retry).toBe(true);

    core.followViewport.mockResolvedValueOnce({
      error: { code: 'host_service_failed', origin: 'host' },
      status: 'error',
    });
    await session.open();
    expect(session.loading).toBe(false);
    expect(session.failure?.retry).toBe(true);

    await session.open();
    ready('recovered', 2);
    await vi.waitFor(() => expect(session.loading).toBe(false));
    expect(session.failure).toBeNull();
  });
});
