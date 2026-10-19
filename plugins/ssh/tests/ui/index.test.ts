// The SSH plugin's entries in the shell (plugins/ssh/docs/README.md;
// ADR-0030, decision 11): the dialogs, "Add SSH host…", the SSH settings
// section and the kind of mentions show only while the plugin is on, the
// list of hosts follows the plugin from its registration, and the Plugins
// page words the plugin's start failure.
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { describe, expect, it, vi } from 'vitest';

import plugin from '../../ui/index.js';

const list = vi.hoisted(() => ({ follow: vi.fn() }));
const navigation = vi.hoisted(() => ({ openSettings: vi.fn(() => Promise.resolve()) }));
const actions = vi.hoisted(() => ({ startAdd: vi.fn() }));

// The list follows the plugin's channel, which these tests do not open.
vi.mock('../../ui/state/hosts.svelte.js', () => ({ hostList: list }));
vi.mock('../../ui/state/ssh-actions.svelte.js', () => ({ SETTINGS_SECTION: 'ssh.settings', sshActions: actions }));
vi.mock('@mannie/plugin-sdk/navigation', () => navigation);

describe('the SSH plugin', () => {
  it('registers its entries, which show only while the plugin is on', () => {
    const context = new TestPluginContext('ssh');

    plugin.register(context);
    const whileOff = [context.shell.commands.entries, context.shell.settingsSections.entries];
    context.turnOn();

    expect(whileOff).toEqual([[], []]);
    expect(context.shell.overlays.entries.map((overlay) => overlay.id)).toEqual(['ssh.dialogs']);
    expect(context.shell.commands.entries.map((command) => command.id)).toEqual(['ssh.add-host']);
    expect(context.shell.settingsSections.entries.map((section) => section.id)).toEqual(['ssh.settings']);
    expect(context.shell.mentionKinds.entries.map((kind) => kind.id)).toEqual(['ssh']);
    expect(list.follow).toHaveBeenCalledWith(context);
  });

  it('opens the settings with the host dialog from the command palette', async () => {
    const context = new TestPluginContext('ssh', true);
    plugin.register(context);
    const [command] = context.shell.commands.entries;

    command.run();
    await Promise.resolve();

    expect(navigation.openSettings).toHaveBeenCalledWith('ssh.settings');
    expect(actions.startAdd).toHaveBeenCalled();
  });

  it('words its mentions and its start failure', () => {
    const context = new TestPluginContext('ssh', true);
    plugin.register(context);
    const [kind] = context.shell.mentionKinds.entries;

    expect(kind.variantName('host')).not.toBeNull();
    expect(kind.variantName('file')).toBeNull();
    expect(kind.warning('not_ready')).not.toBeNull();
    expect(kind.refusal('host_not_found')).not.toBeNull();
    expect(kind.summary({ code: 'host', values: {} })).not.toBeNull();
    expect(plugin.failure?.('storage_failed')).not.toBeNull();
    expect(plugin.failure?.('other')).toBeNull();
  });
});
