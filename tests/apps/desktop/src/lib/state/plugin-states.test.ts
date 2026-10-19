// The plugins as the core reports them (docs/features/plugins.md): the list
// follows the core's channel, a plugin counts as on only while the core
// reports it as on, a change that the core returns shows before its update
// arrives, and a plugin's UI state hears when its plugin turns on and off
// (ADR-0030, decision 11).
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  PluginListUpdate,
  PluginState,
  PluginView,
} from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { PluginStates } from '../../../../../../apps/desktop/src/lib/state/plugin-states.svelte.js';
import type * as Calls from '../../../../../../apps/desktop/src/lib/utils/calls.js';

const core = vi.hoisted(() => ({
  pluginWork: vi.fn(),
  setPluginEnabled: vi.fn(),
  subscribePlugins: vi.fn(),
  unsubscribe: vi.fn(),
}));

/** Hands batches of updates to the list, as the core's channel does. */
const channel = vi.hoisted(() => ({ deliver: (_updates: readonly PluginListUpdate[]): void => undefined }));

vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));
vi.mock('../../../../../../apps/desktop/src/lib/utils/calls.js', async (importOriginal) => ({
  ...(await importOriginal<typeof Calls>()),
  batchChannel: (onItems: (updates: readonly PluginListUpdate[]) => void) => {
    channel.deliver = onItems;
    return {};
  },
}));

function plugin(state: PluginState, id = 'workers'): PluginView {
  return {
    enabled: state !== 'off',
    failure: state === 'failed' ? 'storage_failed' : null,
    id,
    state,
    uses: { channels: [], commands: [], crates: [], native: [], programs: [], secrets: [], services: [] },
  };
}

/** Delivers the whole list with the plugins in these states. */
function deliver(...plugins: PluginView[]): void {
  channel.deliver([{ plugins, type: 'plugins' }]);
}

/** Starts following the list, once the core accepted the subscription. */
async function started(): Promise<PluginStates> {
  const states = new PluginStates();
  states.start();
  await vi.waitFor(() => {
    expect(core.subscribePlugins).toHaveBeenCalled();
  });
  return states;
}

beforeEach(() => {
  for (const command of Object.values(core)) {
    command.mockReset();
  }
  core.subscribePlugins.mockResolvedValue({ data: 3, status: 'ok' });
});

describe('the list of plugins', () => {
  it('loads until the core sends the plugins, then counts a plugin as on only while the core reports it as on', async () => {
    const states = await started();
    const loading = states.status;
    const onWhileLoading = states.isOn('workers');

    deliver(plugin('starting'));
    const onWhileStarting = states.isOn('workers');
    deliver(plugin('on'));

    expect(loading).toBe('loading');
    expect(onWhileLoading).toBe(false);
    expect(onWhileStarting).toBe(false);
    expect(states.status).toBe('ready');
    expect(states.isOn('workers')).toBe(true);
    expect(states.isOn('removed')).toBe(false);
    expect(states.view('workers')?.state).toBe('on');
  });

  it('says why it could not follow the list, and follows it again on request', async () => {
    core.subscribePlugins.mockResolvedValueOnce({ error: { code: 'store_unavailable' }, status: 'error' });
    const states = new PluginStates();
    states.start();
    await vi.waitFor(() => {
      expect(states.status).toBe('error');
    });

    states.retry();
    deliver(plugin('off'));

    expect(states.status).toBe('ready');
    expect(states.failure).toBeNull();
    expect(core.subscribePlugins).toHaveBeenCalledTimes(2);
  });

  it('follows the list again when it fell behind', async () => {
    await started();

    channel.deliver([{ reason: 'lagged', type: 'ended' }]);

    await vi.waitFor(() => {
      expect(core.subscribePlugins).toHaveBeenCalledTimes(2);
    });
  });

  it('shows the plugin that a change returned before its update arrives, and returns why a change failed', async () => {
    const states = await started();
    deliver(plugin('off'));
    core.setPluginEnabled
      .mockResolvedValueOnce({ data: plugin('on'), status: 'ok' })
      .mockResolvedValueOnce({ error: { code: 'storage_failed' }, status: 'error' });

    const turnedOn = await states.setEnabled('workers', true);
    const refused = await states.setEnabled('workers', false);

    expect(core.setPluginEnabled).toHaveBeenNthCalledWith(1, { enabled: true, pluginId: 'workers' });
    expect(turnedOn).toBeNull();
    expect(refused).toEqual({ error: { code: 'storage_failed' }, kind: 'core' });
    expect(states.isOn('workers')).toBe(true);
  });

  it('reads the work that a plugin has in progress', async () => {
    core.pluginWork.mockResolvedValue({ data: { items: [{ code: 'active_tasks', count: 2 }] }, status: 'ok' });
    const states = new PluginStates();

    const work = await states.work('workers');

    expect(core.pluginWork).toHaveBeenCalledWith({ pluginId: 'workers' });
    expect(work).toEqual({ data: { items: [{ code: 'active_tasks', count: 2 }] }, ok: true });
  });
});

describe('following a plugin', () => {
  it('hears each time the plugin turns on and stops being on, and nothing else', async () => {
    const states = await started();
    const on = vi.fn();
    const off = vi.fn();
    states.whileOn('workers', on, off);

    deliver(plugin('off'));
    deliver(plugin('starting'));
    deliver(plugin('on'));
    deliver(plugin('on'), plugin('starting', 'jira'));
    const afterOn = [on.mock.calls.length, off.mock.calls.length];
    deliver(plugin('stopping'));
    deliver(plugin('off'));
    const afterOff = [on.mock.calls.length, off.mock.calls.length];
    deliver(plugin('on'));

    expect(afterOn).toEqual([1, 0]);
    expect(afterOff).toEqual([1, 1]);
    expect([on.mock.calls.length, off.mock.calls.length]).toEqual([2, 1]);
  });

  it('hears at once that a plugin is on, and hears a change that a command returned', async () => {
    const states = await started();
    deliver(plugin('on'));
    core.setPluginEnabled.mockResolvedValue({ data: plugin('stopping'), status: 'ok' });
    const on = vi.fn();
    const off = vi.fn();

    states.whileOn('workers', on, off);
    const atOnce = on.mock.calls.length;
    await states.setEnabled('workers', false);

    expect(atOnce).toBe(1);
    expect(off).toHaveBeenCalledTimes(1);
  });

  it('hears nothing more once it stopped following', async () => {
    const states = await started();
    const on = vi.fn();
    const stop = states.whileOn('workers', on, vi.fn());

    stop();
    deliver(plugin('on'));

    expect(on).not.toHaveBeenCalled();
  });
});
