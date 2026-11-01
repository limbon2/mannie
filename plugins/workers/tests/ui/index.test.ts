// The Workers plugin's entries in the shell (plugins/workers/docs/plugin.md;
// ADR-0030, decision 11): the sidebar group, the dialogs, "Pair a worker…",
// the Workers settings section and the kind of task of its executor show
// only while Workers is on, the list of workers follows the plugin from its
// registration, and the Plugins page asks the plugin's own questions. The
// entries of its feature Databases show only while Workers and Databases
// are on, and the list of databases follows the feature (ADR-0055).
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { describe, expect, it, vi } from 'vitest';

import plugin from '../../ui/index.js';
import { workersOffQuestion, workersStartFailureMessage } from '../../ui/utils/workers-messages.js';

const list = vi.hoisted(() => ({ follow: vi.fn() }));
const databases = vi.hoisted(() => ({ find: vi.fn(), follow: vi.fn() }));

// The lists follow the plugin's channels, which these tests do not open.
vi.mock('../../ui/state/workers.svelte.js', () => ({ workerList: list }));
vi.mock('../../ui/state/databases.svelte.js', () => ({ databaseList: databases }));
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

  it('registers the entries of Databases, which show only while Workers and Databases are on', () => {
    const context = new TestPluginContext('workers');

    plugin.register(context);
    context.turnOn();
    const whileOff = [
      context.shell.navigation.entries,
      context.shell.views.entries,
      context.shell.mentionKinds.entries,
      context.shell.toolKinds.entries,
    ];
    context.feature('databases').turnOn();

    expect(whileOff).toEqual([[], [], [], []]);
    expect(context.shell.navigation.entries.map((entry) => entry.id)).toEqual(['workers.databases']);
    expect(context.shell.views.entries.map((view) => view.id)).toEqual(['workers.databases']);
    expect(context.shell.sidebarPanels.entries.map((panel) => panel.id)).toEqual([
      'workers.panel',
      'workers.databases.panel',
    ]);
    expect(context.shell.overlays.entries.map((overlay) => overlay.id)).toEqual([
      'workers.dialogs',
      'workers.databases.dialogs',
    ]);
    expect(context.shell.mentionKinds.entries.map((kind) => kind.id)).toEqual(['workers.databases']);
    expect(context.shell.toolKinds.entries.map((kind) => kind.id)).toEqual([
      'describe_database',
      'query_database',
      'change_database',
    ]);
    expect(context.shell.commands.entries.map((command) => command.id)).toEqual([
      'workers.pair',
      'workers.databases.add',
    ]);
    expect(databases.follow).toHaveBeenCalledWith(context.feature('databases'));
  });

  it("words the failure of Databases and asks before it lets the view's statements go", () => {
    expect(workersStartFailureMessage('databases_unavailable')).not.toBeNull();
    expect(workersOffQuestion([{ code: 'running_statements', count: 2 }])).not.toBeNull();
    expect(workersOffQuestion([])).toBeNull();
  });
});
