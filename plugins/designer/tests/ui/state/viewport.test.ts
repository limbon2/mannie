import type * as Ipc from '@mannie/plugin-sdk/ipc';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { CommitViewportColor, DocumentSnapshot, GraphicsEvent } from '../../../ui/generated/bindings.js';
import { ProjectViewport } from '../../../ui/state/viewport.svelte.js';

type Deliver = (items: readonly Ipc.ChannelMessage<GraphicsEvent>[]) => void;
const core = vi.hoisted(() => ({
  deliver: [] as Deliver[],
  commitViewportColor: vi.fn(),
  followViewport: vi.fn(),
  placeViewport: vi.fn(),
  readViewportDocument: vi.fn(),
  setViewportColor: vi.fn(),
  setViewportSelection: vi.fn(),
  unsubscribe: vi.fn(),
}));

vi.mock('../../../ui/generated/bindings.js', () => ({
  channels: { followViewport: core.followViewport },
  commands: {
    commitViewportColor: core.commitViewportColor,
    placeViewport: core.placeViewport,
    readViewportDocument: core.readViewportDocument,
    setViewportColor: core.setViewportColor,
    setViewportSelection: core.setViewportSelection,
  },
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

function snapshot(hue = 0, revision = 1): DocumentSnapshot {
  return {
    element: {
      height: 160,
      hue,
      id: 'rectangle',
      width: 240,
      x: 64,
      y: 64,
    },
    revision,
  };
}

function readResult(hue = 0, revision = 1) {
  return { data: snapshot(hue, revision), status: 'ok' };
}

function storeFailure(confirmed: DocumentSnapshot | null) {
  return { error: { code: 'storage_failed', confirmed, origin: 'plugin' }, status: 'error' };
}

function ready(lease: string, channel = 0): void {
  core.deliver[channel]?.([{ item: { kind: 'ready', lease }, kind: 'item' }]);
}

beforeEach(() => {
  core.deliver.length = 0;
  vi.resetAllMocks();
  core.followViewport.mockResolvedValue({ data: 2, status: 'ok' });
  core.setViewportColor.mockResolvedValue(ok);
  core.setViewportSelection.mockResolvedValue(ok);
  core.placeViewport.mockResolvedValue(ok);
  core.readViewportDocument.mockResolvedValue(readResult());
  core.commitViewportColor.mockImplementation((input: CommitViewportColor) =>
    Promise.resolve(readResult(input.hue, input.revision + 1)),
  );
});

describe('the Designer viewport', () => {
  it('coalesces pending selection draws and leaves the document revision untouched', async () => {
    const pending = Promise.withResolvers<typeof ok>();
    core.setViewportSelection.mockReturnValueOnce(pending.promise);
    const session = new ProjectViewport('drawing');
    session.place(shown);
    await session.open();
    ready('current');
    await vi.waitFor(() => expect(session.canSelect).toBe(true));

    session.select({ kind: 'select' });
    session.select({ kind: 'clear' });
    session.select({ kind: 'select' });
    session.select({ kind: 'clear' });
    expect(core.setViewportSelection).toHaveBeenCalledExactlyOnceWith({
      projectId: 'drawing',
      lease: 'current',
      elementId: 'rectangle',
    });
    expect(session.selectedId).toBeNull();
    pending.resolve(ok);
    await vi.waitFor(() => expect(core.setViewportSelection).toHaveBeenCalledTimes(2));
    expect(core.setViewportSelection).toHaveBeenLastCalledWith({
      projectId: 'drawing',
      lease: 'current',
      elementId: null,
    });
    expect(session.selectedId).toBeNull();
    expect(session.confirmed).toEqual(snapshot());
    expect(core.commitViewportColor).not.toHaveBeenCalled();
  });

  it('keeps selection across resize, hue changes, and a covered canvas that refuses input', async () => {
    const session = new ProjectViewport('drawing');
    session.place(shown);
    await session.open();
    ready('current');
    await vi.waitFor(() => expect(session.canSelect).toBe(true));
    session.select({ kind: 'select' });
    await vi.waitFor(() => expect(session.selectedId).toBe('rectangle'));

    session.place({ ...shown, visible: false });
    expect(session.canSelect).toBe(false);
    session.select({ kind: 'clear' });
    session.place({ ...shown, bounds: { ...rect, width: 260 } });
    await vi.waitFor(() => expect(session.canSelect).toBe(true));
    session.commitHue(120);
    await vi.waitFor(() => expect(session.confirmed?.element.hue).toBe(120));
    expect(core.setViewportSelection).toHaveBeenCalledTimes(1);
    expect(session.selectedId).toBe('rectangle');
  });

  it('uses pointer input only at the acknowledged placement while keeping the keyboard available during resize', async () => {
    const pending = Promise.withResolvers<typeof ok>();
    const latest = Promise.withResolvers<typeof ok>();
    const session = new ProjectViewport('drawing');
    const bounds = { ...rect, height: 160 };
    const initial = { ...shown, bounds, clip: bounds };
    session.place(initial);
    await session.open();
    ready('current');
    await vi.waitFor(() => expect(session.canSelect).toBe(true));
    session.select({ kind: 'select' });
    await vi.waitFor(() => expect(session.selectedId).toBe('rectangle'));

    const moved = { ...bounds, x: rect.x + 5 };
    const resized = { ...moved, width: moved.width + 10 };
    const client = { x: 30, y: 30 };
    // The DOM moved before its observer has measured the new bounds.
    session.select({ kind: 'point', client, bounds: moved });
    expect(core.setViewportSelection).toHaveBeenCalledTimes(1);

    core.placeViewport.mockReturnValueOnce(pending.promise).mockReturnValueOnce(latest.promise);
    session.place({ ...initial, bounds: moved, clip: moved });
    session.select({ kind: 'point', client, bounds: moved });
    session.place({ ...initial, bounds: resized, clip: resized });
    pending.resolve(ok);
    await vi.waitFor(() => expect(core.placeViewport).toHaveBeenCalledTimes(3));
    session.select({ kind: 'point', client, bounds: resized });
    expect(session.canSelect).toBe(true);
    session.select({ kind: 'clear' });
    // This pointer hit would restore the rectangle if the stale acknowledgement enabled pointing.
    session.select({ kind: 'point', client: { x: 85, y: 90 }, bounds: resized });
    latest.resolve(ok);
    await vi.waitFor(() => expect(session.selectedId).toBeNull());
    expect(core.setViewportSelection).toHaveBeenCalledTimes(2);
    session.select({ kind: 'select' });
    await vi.waitFor(() => expect(session.selectedId).toBe('rectangle'));
    session.select({ kind: 'point', client, bounds: resized });
    await vi.waitFor(() => expect(session.selectedId).toBeNull());
  });

  it('disables selection on a drawing failure, reopens clear and ignores a late draw from the closed lease', async () => {
    const session = new ProjectViewport('drawing');
    session.place(shown);
    await session.open();
    ready('first');
    await vi.waitFor(() => expect(session.canSelect).toBe(true));
    core.setViewportSelection.mockResolvedValueOnce({
      status: 'error',
      error: { origin: 'plugin', code: 'render_failed' },
    });
    session.select({ kind: 'select' });
    await vi.waitFor(() => expect(session.failure?.retry).toBe(true));
    expect(session.selectedId).toBeNull();
    expect(session.canSelect).toBe(false);

    await session.open();
    ready('second', 1);
    await vi.waitFor(() => expect(session.canSelect).toBe(true));
    const pending = Promise.withResolvers<typeof ok>();
    core.setViewportSelection.mockReturnValueOnce(pending.promise);
    session.select({ kind: 'select' });
    session.close();
    pending.resolve(ok);
    await pending.promise;
    expect(session.selectedId).toBeNull();
    expect(session.canSelect).toBe(false);
  });

  it('restores the project color before showing the native scene and keeps projects separate', async () => {
    core.readViewportDocument.mockResolvedValueOnce(readResult(360));
    const session = new ProjectViewport('color');
    session.place(shown);
    await session.open();
    ready('first');

    await vi.waitFor(() => expect(session.loading).toBe(false));
    expect(session.hue).toBe(360);
    expect(core.setViewportColor).not.toHaveBeenCalled();
    expect(core.commitViewportColor).not.toHaveBeenCalled();
    expect(core.placeViewport).toHaveBeenCalledWith({ lease: 'first', placement: shown, projectId: 'color' });
    expect(core.readViewportDocument.mock.invocationCallOrder[0]).toBeLessThan(
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
    await vi.waitFor(() => expect(session.canEdit).toBe(true));
    session.changeHue(60);
    session.changeHue(120);
    session.changeHue(240);
    paused.resolve(ok);

    await vi.waitFor(() => expect(core.setViewportColor).toHaveBeenCalledTimes(2));
    expect(core.setViewportColor).toHaveBeenNthCalledWith(2, { hue: 240, lease: 'dragging', projectId: 'drag' });
    expect(core.commitViewportColor).not.toHaveBeenCalled();
    session.commitHue(240);
    await vi.waitFor(() => expect(session.confirmed?.element.hue).toBe(240));
    expect(core.commitViewportColor).toHaveBeenCalledTimes(1);
    session.changeHue(90);
    session.place({ ...shown, visible: false });
    await vi.waitFor(() =>
      expect(core.setViewportColor).toHaveBeenLastCalledWith({ hue: 240, lease: 'dragging', projectId: 'drag' }),
    );
    expect(session.hue).toBe(240);
    expect(core.commitViewportColor).toHaveBeenCalledTimes(1);
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
    expect(core.readViewportDocument).toHaveBeenCalledExactlyOnceWith({ lease: 'current', projectId: 'new' });
  });

  it('restores the confirmed color on a failed save and retries it once against the same revision', async () => {
    core.readViewportDocument.mockResolvedValue(readResult(92, 4));
    core.commitViewportColor.mockResolvedValueOnce(storeFailure(snapshot(92, 4)));
    const session = new ProjectViewport('retry-save');
    await session.open();
    ready('surface');
    await vi.waitFor(() => expect(session.canEdit).toBe(true));
    session.changeHue(360);
    session.commitHue(360);
    await vi.waitFor(() => expect(session.documentError).not.toBeNull());
    expect(session.hue).toBe(92);
    expect(session.canEdit).toBe(false);
    expect(core.unsubscribe).not.toHaveBeenCalled();
    session.retryDocument();
    await vi.waitFor(() => expect(session.confirmed).toEqual(snapshot(360, 5)));
    expect(core.readViewportDocument).toHaveBeenCalledTimes(2);
    expect(core.commitViewportColor).toHaveBeenCalledTimes(2);
    expect(session.documentError).toBeNull();
  });

  it('keeps the cached drawing on a failed read and retries without writing', async () => {
    core.readViewportDocument.mockResolvedValueOnce(storeFailure(snapshot(87, 8)));
    core.readViewportDocument.mockResolvedValueOnce(readResult(100, 9));
    const session = new ProjectViewport('retry-read');
    await session.open();
    ready('surface');
    await vi.waitFor(() => expect(session.documentError).not.toBeNull());
    expect(session.hue).toBe(87);
    session.retryDocument();
    await vi.waitFor(() => expect(session.canEdit).toBe(true));
    expect(session.hue).toBe(100);
    expect(core.commitViewportColor).not.toHaveBeenCalled();
  });

  it('offers viewport recovery if graphics fail while a document retry is available', async () => {
    core.readViewportDocument.mockResolvedValueOnce(storeFailure(null));
    const session = new ProjectViewport('unopened');
    await session.open();
    ready('surface');
    await vi.waitFor(() => expect(session.documentError).not.toBeNull());
    expect(session.confirmed).toBeNull();
    core.deliver[0]?.([{ item: { error: { code: 'render_failed' }, kind: 'failed' }, kind: 'item' }]);
    expect(session.documentError).toBeNull();
    expect(session.failure?.retry).toBe(true);
    expect(core.commitViewportColor).not.toHaveBeenCalled();
  });

  it('keeps rapid keyboard commits ordered and discards a failed edit if the document changed before retry', async () => {
    const first = Promise.withResolvers<ReturnType<typeof readResult>>();
    core.commitViewportColor.mockReturnValueOnce(first.promise);
    core.commitViewportColor.mockResolvedValueOnce(storeFailure(snapshot(1, 2)));
    const session = new ProjectViewport('keyboard');
    await session.open();
    ready('surface');
    await vi.waitFor(() => expect(session.canEdit).toBe(true));
    session.commitHue(1);
    session.commitHue(2);
    first.resolve(readResult(1, 2));
    await vi.waitFor(() => expect(session.documentError).not.toBeNull());
    expect(core.commitViewportColor).toHaveBeenNthCalledWith(1, expect.objectContaining({ hue: 1, revision: 1 }));
    expect(core.commitViewportColor).toHaveBeenNthCalledWith(2, expect.objectContaining({ hue: 2, revision: 2 }));
    core.readViewportDocument.mockResolvedValueOnce(readResult(200, 3));
    session.retryDocument();
    await vi.waitFor(() => expect(session.canEdit).toBe(true));
    expect(session.hue).toBe(200);
    expect(core.commitViewportColor).toHaveBeenCalledTimes(2);
  });

  it('finishes a released slider edit before switching projects even when its preview is still in flight', async () => {
    const preview = Promise.withResolvers<typeof ok>();
    core.setViewportColor.mockReturnValueOnce(preview.promise);
    const first = new ProjectViewport('first');
    await first.open();
    ready('old');
    await vi.waitFor(() => expect(first.canEdit).toBe(true));
    first.changeHue(45);
    first.commitHue(45);
    first.dispose();
    const next = new ProjectViewport('second');
    const opening = next.open();
    expect(core.unsubscribe).not.toHaveBeenCalled();
    preview.resolve(ok);
    await opening;
    expect(core.commitViewportColor).toHaveBeenCalledExactlyOnceWith({
      projectId: 'first',
      elementId: 'rectangle',
      lease: 'old',
      revision: 1,
      hue: 45,
    });
    expect(core.commitViewportColor.mock.invocationCallOrder[0]).toBeLessThan(
      core.unsubscribe.mock.invocationCallOrder[0],
    );
    expect(core.followViewport).toHaveBeenLastCalledWith({ projectId: 'second' }, expect.anything());
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
    expect(core.readViewportDocument).toHaveBeenCalledTimes(2);
    expect(core.readViewportDocument).toHaveBeenLastCalledWith({ lease: 'new', projectId: 'retry' });
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
