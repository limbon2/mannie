// The plugin's entries in the shell (ADR-0030, decision 11): its side panel,
// its toolbar control, its palette command with Ctrl+`, its settings
// section, its tools' cards, its kind of mentions and its kind of context
// show only while the plugin is on, and the terminals follow the plugin from
// its registration.
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { describe, expect, it, vi } from 'vitest';

import plugin from '../../ui/index.js';

const state = vi.hoisted(() => ({
  terminals: { follow: vi.fn(), of: vi.fn(() => []), open: vi.fn(), reveal: vi.fn() },
}));

// The terminals follow the plugin's channels, which these tests do not open,
// and xterm.js loads only in a terminal's panel.
vi.mock('../../ui/state/terminals.svelte.js', () => ({ terminals: state.terminals }));
vi.mock('@mannie/plugin-sdk/terminal', () => ({ createTerminalView: vi.fn() }));
vi.mock('@mannie/plugin-sdk/navigation', () => ({
  openConversationId: vi.fn(() => 'c1'),
  openSettings: vi.fn(),
  openSidePanel: vi.fn(),
}));

describe('the plugin', () => {
  it('registers its entries, which show only while it is on', () => {
    const context = new TestPluginContext('terminal');

    plugin.register(context);
    const whileOff = [context.shell.sidePanels.entries, context.shell.commands.entries];
    context.turnOn();

    expect(whileOff).toEqual([[], []]);
    expect(context.shell.sidePanels.entries.map((panel) => panel.id)).toEqual(['terminal.session']);
    expect(context.shell.conversationControls.entries.map((control) => control.id)).toEqual(['terminal.control']);
    const commands = context.shell.commands.entries;
    expect(commands.map((command) => command.id)).toEqual(['terminal.open']);
    expect(commands.map((command) => command.key?.code)).toEqual(['Backquote']);
    expect(commands.map((command) => command.available?.())).toEqual([true]);
    expect(context.shell.settingsSections.entries.map((section) => section.id)).toEqual(['terminal.settings']);
    expect(context.shell.toolKinds.entries.map((kind) => kind.id)).toEqual(['run_in_terminal', 'list_terminals']);
    expect(context.shell.mentionKinds.entries.map((kind) => kind.id)).toEqual(['terminal']);
    expect(context.shell.contextKinds.entries.map((kind) => kind.id)).toEqual(['terminal']);
    expect(state.terminals.follow).toHaveBeenCalledWith(context);
  });

  it('words its start failure and the work that turning it off ends', () => {
    expect(plugin.failure?.('storage_failed')).toMatch(/settings/u);
    expect(plugin.failure?.('other')).toBeNull();
    expect(plugin.offQuestion?.([])).toBeNull();
    expect(plugin.offQuestion?.([{ code: 'running_commands', count: 2 }])).toMatch(/2 terminals/u);
  });
});
