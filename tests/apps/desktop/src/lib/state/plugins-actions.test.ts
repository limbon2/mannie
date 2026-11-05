// What the Plugins page does (docs/features/plugins.md): a switch takes effect
// at once, turning off a plugin with work in progress asks first and says
// what happens to that work, a failed change stays beside the plugin until the
// next one, and "Try again" starts a failed plugin again. A plugin's optional
// feature does the same under its own key (ADR-0055, decision 10).
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PluginView } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { setPluginCatalog } from '../../../../../../apps/desktop/src/lib/state/plugin-catalog.js';
import { featureKey, PluginsActions } from '../../../../../../apps/desktop/src/lib/state/plugins-actions.svelte.js';

const core = vi.hoisted(() => ({
  pluginFeatureWork: vi.fn(),
  pluginWork: vi.fn(),
  setPluginEnabled: vi.fn(),
  setPluginFeatureEnabled: vi.fn(),
  subscribePlugins: vi.fn(),
  unsubscribe: vi.fn(),
}));

vi.mock('../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: core }));

function view(enabled: boolean): PluginView {
  return {
    enabled,
    failure: null,
    features: [],
    id: 'workers',
    state: enabled ? 'on' : 'off',
    uses: {
      channels: [],
      commands: [],
      crates: [],
      installedPrograms: [],
      native: [],
      programs: [],
      secrets: [],
      services: [],
    },
  };
}

beforeEach(() => {
  for (const command of Object.values(core)) {
    command.mockReset();
  }
  core.setPluginEnabled.mockImplementation(({ enabled }: { enabled: boolean }) =>
    Promise.resolve({ data: view(enabled), status: 'ok' }),
  );
  core.setPluginFeatureEnabled.mockResolvedValue({ data: view(true), status: 'ok' });
  setPluginCatalog([
    {
      description: () => 'Runs tasks on other computers.',
      failure: () => null,
      features: [],
      id: 'workers',
      name: () => 'Workers',
      offQuestion: (items) => {
        const count = items.find((item) => item.code === 'active_tasks')?.count ?? 0;
        return count === 0 ? null : `${String(count)} tasks keep running.`;
      },
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
    core.pluginWork.mockResolvedValue({ data: { items: [{ code: 'active_tasks', count: 2 }] }, status: 'ok' });
    const actions = new PluginsActions();

    await actions.toggle('workers', false);
    const asked = { open: actions.offOpen, question: actions.offQuestion };
    const turnedOffBeforeConfirming = core.setPluginEnabled.mock.calls.length;
    await actions.confirmOff();

    expect(asked).toEqual({
      open: true,
      question: { feature: null, id: 'workers', question: '2 tasks keep running.' },
    });
    expect(turnedOffBeforeConfirming).toBe(0);
    expect(core.setPluginEnabled).toHaveBeenCalledWith({ enabled: false, pluginId: 'workers' });
    expect(actions.offOpen).toBe(false);
    expect(actions.offQuestion).toBeNull();
  });

  it('names the commands that its tools started in the background beside its own work', async () => {
    core.pluginWork.mockResolvedValue({
      data: {
        items: [
          { code: 'active_tasks', count: 1 },
          { code: 'core.background_commands', count: 1 },
        ],
      },
      status: 'ok',
    });
    const actions = new PluginsActions();

    await actions.toggle('workers', false);

    expect(actions.offQuestion).toEqual({
      feature: null,
      id: 'workers',
      question:
        '1 tasks keep running. Its tools started a command in the background that still runs. Turning the plugin off stops it.',
    });
  });

  it('turns off a plugin without work at once, and one whose work cannot be read', async () => {
    core.pluginWork
      .mockResolvedValueOnce({ data: { items: [] }, status: 'ok' })
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

  it('turns a feature on at once, under its own key', async () => {
    const actions = new PluginsActions();

    const turning = actions.toggleFeature('workers', 'databases', true);
    const changing = actions.changing.has(featureKey('workers', 'databases'));
    await turning;

    expect(core.setPluginFeatureEnabled).toHaveBeenCalledWith({
      enabled: true,
      featureId: 'databases',
      pluginId: 'workers',
    });
    expect(changing).toBe(true);
    expect(actions.changing.has('workers')).toBe(false);
    expect(actions.changing.size).toBe(0);
  });

  it('asks before turning off a feature with work in progress, in the plugin’s words, and turns it off once confirmed', async () => {
    core.pluginFeatureWork.mockResolvedValue({ data: { items: [{ code: 'active_tasks', count: 3 }] }, status: 'ok' });
    const actions = new PluginsActions();

    await actions.toggleFeature('workers', 'databases', false);
    const asked = actions.offQuestion;
    await actions.confirmOff();

    expect(core.pluginFeatureWork).toHaveBeenCalledWith({ featureId: 'databases', pluginId: 'workers' });
    expect(asked).toEqual({ feature: 'databases', id: 'workers', question: '3 tasks keep running.' });
    expect(core.setPluginFeatureEnabled).toHaveBeenCalledWith({
      enabled: false,
      featureId: 'databases',
      pluginId: 'workers',
    });
    expect(core.setPluginEnabled).not.toHaveBeenCalled();
  });

  it('keeps why a feature’s change failed beside the feature, and tries its start again', async () => {
    core.pluginFeatureWork.mockResolvedValue({ data: { items: [] }, status: 'ok' });
    core.setPluginFeatureEnabled.mockResolvedValueOnce({ error: { code: 'storage_failed' }, status: 'error' });
    const actions = new PluginsActions();

    await actions.toggleFeature('workers', 'databases', false);
    const failure = actions.failures.get(featureKey('workers', 'databases'));
    await actions.retryFeature('workers', 'databases');

    expect(failure).toEqual({ error: { code: 'storage_failed' }, kind: 'core' });
    expect(actions.failures.has('workers')).toBe(false);
    expect(actions.failures.size).toBe(0);
    expect(core.setPluginFeatureEnabled).toHaveBeenLastCalledWith({
      enabled: true,
      featureId: 'databases',
      pluginId: 'workers',
    });
  });
});
