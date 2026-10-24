// The MCP plugin's entries in the shell (plugins/mcp/docs/README.md;
// ADR-0030, decision 11): the dialogs, "Add MCP server…", the MCP settings
// section, the kind of mentions and the one entry that describes every tool
// of the servers show only while the plugin is on, the list of servers
// follows the plugin from its registration, and the Plugins page words the
// plugin's start failures and asks before it ends calls in progress.
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { describe, expect, it, vi } from 'vitest';

import plugin from '../../ui/index.js';

const list = vi.hoisted(() => ({ follow: vi.fn() }));
const navigation = vi.hoisted(() => ({ openSettings: vi.fn(() => Promise.resolve()) }));
const actions = vi.hoisted(() => ({ startAdd: vi.fn() }));

// The list follows the plugin's channel, which these tests do not open.
vi.mock('../../ui/state/servers.svelte.js', () => ({ serverList: list }));
vi.mock('../../ui/state/mcp-actions.svelte.js', () => ({ mcpActions: actions, SETTINGS_SECTION: 'mcp.settings' }));
vi.mock('@mannie/plugin-sdk/navigation', () => navigation);

describe('the MCP plugin', () => {
  it('registers its entries, which show only while the plugin is on', () => {
    const context = new TestPluginContext('mcp');

    plugin.register(context);
    const whileOff = [context.shell.commands.entries, context.shell.settingsSections.entries];
    context.turnOn();

    expect(whileOff).toEqual([[], []]);
    expect(context.shell.overlays.entries.map((overlay) => overlay.id)).toEqual(['mcp.dialogs']);
    expect(context.shell.commands.entries.map((command) => command.id)).toEqual(['mcp.add-server']);
    expect(context.shell.settingsSections.entries.map((section) => section.id)).toEqual(['mcp.settings']);
    expect(context.shell.mentionKinds.entries.map((kind) => kind.id)).toEqual(['mcp']);
    expect(context.shell.toolKinds.entries.map((kind) => [kind.id, kind.everyTool])).toEqual([['mcp.servers', true]]);
    expect(list.follow).toHaveBeenCalledWith(context);
  });

  it('opens the settings and the server dialog from the command palette', async () => {
    const context = new TestPluginContext('mcp', true);
    plugin.register(context);
    const [command] = context.shell.commands.entries;

    command.run();
    await Promise.resolve();
    await Promise.resolve();

    expect(navigation.openSettings).toHaveBeenCalledWith('mcp.settings');
    expect(actions.startAdd).toHaveBeenCalledOnce();
  });

  it('words its tools failures and leaves their lines to the core', () => {
    const context = new TestPluginContext('mcp', true);
    plugin.register(context);
    const [kind] = context.shell.toolKinds.entries;

    expect(kind.failure('server_stopped')).toBe(
      'The server stopped during the call, and whether the call took effect is unknown.',
    );
    expect(kind.failure('unknown')).toBeNull();
    expect(kind.summary({ arguments: {}, data: null, failure: null, status: 'completed' })).toBeNull();
  });

  it('words its start failures and asks before it ends calls in progress', () => {
    expect(plugin.failure?.('storage_failed')).toContain('could not read its servers');
    expect(plugin.failure?.('plugin_panicked')).toBeNull();
    expect(plugin.offQuestion?.([])).toBeNull();
    expect(plugin.offQuestion?.([{ code: 'connected_servers', count: 2 }])).toBeNull();
    expect(plugin.offQuestion?.([{ code: 'active_calls', count: 2 }])).toBe(
      '2 calls of MCP tools are running. Turning MCP off ends them, and whether they took effect is unknown.',
    );
  });
});
