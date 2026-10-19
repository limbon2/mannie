// The Workers plugin's entries in the shell (plugins/workers/docs/plugin.md;
// ADR-0030, decision 11): the sidebar group, the dialogs, "Pair a worker…",
// the Workers settings section and the kind of task of its executor show
// only while Workers is on, the list of workers follows the plugin from its
// registration, and the Plugins page asks the plugin's own questions.
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { describe, expect, it, vi } from 'vitest';

import plugin from '../../ui/index.js';
import { workersOffQuestion, workersStartFailureMessage } from '../../ui/utils/workers-messages.js';

const list = vi.hoisted(() => ({ follow: vi.fn() }));

// The list follows the plugin's channel, which these tests do not open.
vi.mock('../../ui/state/workers.svelte.js', () => ({ workerList: list }));
vi.mock('@mannie/plugin-sdk/navigation', () => ({ openSettings: vi.fn() }));

describe('the Workers plugin', () => {
  it('registers its entries, which show only while Workers is on', () => {
    const context = new TestPluginContext('workers');

    plugin.register(context);
    const whileOff = [context.shell.sidebarPanels.entries, context.shell.commands.entries];
    context.turnOn();

    expect(whileOff).toEqual([[], []]);
    expect(context.shell.sidebarPanels.entries.map((panel) => panel.id)).toEqual(['workers.panel']);
    expect(context.shell.overlays.entries.map((overlay) => overlay.id)).toEqual(['workers.dialogs']);
    expect(context.shell.commands.entries.map((command) => command.id)).toEqual(['workers.pair']);
    expect(context.shell.settingsSections.entries.map((section) => section.id)).toEqual(['workers.settings']);
    expect(context.shell.taskKinds.entries.map((kind) => [kind.id, kind.tool])).toEqual([['workers', 'delegate_task']]);
  });

  it('lets the list of workers follow the plugin', () => {
    const context = new TestPluginContext('workers');

    plugin.register(context);

    expect(list.follow).toHaveBeenCalledWith(context);
  });

  it('words its start failures and asks before it lets worker tasks go', () => {
    expect(plugin.failure).toBe(workersStartFailureMessage);
    expect(plugin.offQuestion).toBe(workersOffQuestion);
  });
});
