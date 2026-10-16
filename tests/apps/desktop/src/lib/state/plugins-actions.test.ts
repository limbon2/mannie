// What the Plugins page does (docs/features/plugins.md): a switch takes effect
// at once, turning off a plugin with work in progress asks first and says
// what happens to that work, a failed change stays beside the plugin until the
// next one, and "Try again" starts a failed plugin again.
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PluginView } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { setPluginCatalog } from '../../../../../../apps/desktop/src/lib/state/plugin-catalog.js';
import { PluginsActions } from '../../../../../../apps/desktop/src/lib/state/plugins-actions.svelte.js';

const core = vi.hoisted(() => ({
  pluginWork: vi.fn(),
  setPluginEnabled: vi.fn(),
  subscribePlugins: vi.fn(),
  unsubscribe: vi.fn(),
}));

vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));

function view(enabled: boolean): PluginView {
  return { enabled, failure: null, id: 'workers', state: enabled ? 'on' : 'off' };
}

beforeEach(() => {
  for (const command of Object.values(core)) {
    command.mockReset();
  }
  core.setPluginEnabled.mockImplementation(({ enabled }: { enabled: boolean }) =>
    Promise.resolve({ data: view(enabled), status: 'ok' }),
  );
  setPluginCatalog([
    {
      description: () => 'Runs tasks on other computers.',
      failure: () => null,
      id: 'workers',
      name: () => 'Workers',
      offQuestion: (work) => (work.activeTasks === 0 ? null : `${String(work.activeTasks)} tasks keep running.`),
    },
  ]);
});

afterEach(() => {
  setPluginCatalog([]);
});

describe('the Plugins page', () => {
  it('turns a plugin on at once, and shows the switch as changing until the core answered', async () => {
    const actions = new PluginsActions();

    const turning = actions.toggle('workers', true);
    const changing = actions.changing.has('workers');
    await turning;

    expect(core.setPluginEnabled).toHaveBeenCalledWith({ enabled: true, pluginId: 'workers' });
    expect(changing).toBe(true);
    expect(actions.changing.has('workers')).toBe(false);
    expect(actions.offOpen).toBe(false);
  });

  it('asks before turning off a plugin with work in progress, and turns it off once the owner confirms', async () => {
    core.pluginWork.mockResolvedValue({ data: { activeTasks: 2 }, status: 'ok' });
    const actions = new PluginsActions();

    await actions.toggle('workers', false);
    const asked = { open: actions.offOpen, question: actions.offQuestion };
    const turnedOffBeforeConfirming = core.setPluginEnabled.mock.calls.length;
    await actions.confirmOff();

    expect(asked).toEqual({ open: true, question: { id: 'workers', question: '2 tasks keep running.' } });
    expect(turnedOffBeforeConfirming).toBe(0);
    expect(core.setPluginEnabled).toHaveBeenCalledWith({ enabled: false, pluginId: 'workers' });
    expect(actions.offOpen).toBe(false);
    expect(actions.offQuestion).toBeNull();
  });

  it('turns off a plugin without work at once, and one whose work cannot be read', async () => {
    core.pluginWork
      .mockResolvedValueOnce({ data: { activeTasks: 0 }, status: 'ok' })
      .mockResolvedValueOnce({ error: { code: 'store_unavailable' }, status: 'error' });
    const actions = new PluginsActions();

    await actions.toggle('workers', false);
    await actions.toggle('workers', false);

    expect(core.setPluginEnabled).toHaveBeenCalledTimes(2);
    expect(actions.offOpen).toBe(false);
  });

  it('keeps why a change failed beside the plugin until the next change', async () => {
    core.setPluginEnabled.mockResolvedValueOnce({ error: { code: 'storage_failed' }, status: 'error' });
    const actions = new PluginsActions();

    await actions.toggle('workers', true);
    const failure = actions.failures.get('workers');
    await actions.retry('workers');

    expect(failure).toEqual({ error: { code: 'storage_failed' }, kind: 'core' });
    expect(actions.failures.has('workers')).toBe(false);
    expect(core.setPluginEnabled).toHaveBeenLastCalledWith({ enabled: true, pluginId: 'workers' });
  });
});
