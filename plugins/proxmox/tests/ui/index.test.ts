// Tests for the Proxmox plugin UI registration (ADR-0030, decision 11; ADR-0080).
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { describe, expect, it, vi } from 'vitest';

import plugin from '../../ui/index.js';

const list = vi.hoisted(() => ({ follow: vi.fn() }));
const navigation = vi.hoisted(() => ({
  openConversationId: vi.fn(() => null),
  openSettings: vi.fn(() => Promise.resolve()),
  openSidePanel: vi.fn(() => Promise.resolve()),
  setSidePanelLabel: vi.fn(),
}));
const actions = vi.hoisted(() => ({ startAdd: vi.fn() }));

vi.mock('../../ui/state/servers.svelte.js', () => ({ serverList: list }));
vi.mock('../../ui/state/proxmox-actions.svelte.js', () => ({
  proxmoxActions: actions,
  SETTINGS_SECTION: 'proxmox.settings',
}));
vi.mock('@mannie/plugin-sdk/navigation', () => navigation);

describe('the Proxmox plugin', () => {
  it('registers all slots and exposes them only while the plugin is turned on', () => {
    const context = new TestPluginContext('proxmox');

    plugin.register(context);
    const whileOff = [
      context.shell.commands.entries,
      context.shell.settingsSections.entries,
      context.shell.sidePanels.entries,
      context.shell.conversationControls.entries,
    ];
    context.turnOn();

    expect(whileOff).toEqual([[], [], [], []]);
    expect(context.shell.overlays.entries.map((entry) => entry.id)).toEqual(['proxmox.dialogs']);
    expect(context.shell.commands.entries.map((entry) => entry.id)).toEqual(['proxmox.add-server']);
    expect(context.shell.settingsSections.entries.map((entry) => entry.id)).toEqual(['proxmox.settings']);
    expect(context.shell.sidePanels.entries.map((entry) => entry.id)).toEqual(['proxmox.vnc']);
    expect(context.shell.conversationControls.entries.map((entry) => entry.id)).toEqual(['proxmox.control']);
    expect(context.shell.conversationControls.entries[0]?.place).toBe('menu');
    expect(context.shell.mentionKinds.entries.map((entry) => entry.id)).toEqual(['proxmox']);
    expect(context.shell.toolKinds.entries.map((entry) => entry.id)).toEqual([
      'proxmox_list_vms',
      'proxmox_get_vm_config',
      'proxmox_list_isos',
      'proxmox_attach_iso',
      'proxmox_eject_iso',
      'proxmox_set_boot_order',
      'proxmox_vm_power',
      'proxmox_vnc_screenshot',
      'proxmox_vnc_pointer',
      'proxmox_vnc_keyboard',
      'proxmox_vnc_close',
    ]);
    expect(list.follow).toHaveBeenCalledWith(context);
  });

  it('opens Proxmox settings and starts Add Server from the command palette', async () => {
    const context = new TestPluginContext('proxmox', true);
    plugin.register(context);
    const [command] = context.shell.commands.entries;

    command.run();
    await Promise.resolve();

    expect(navigation.openSettings).toHaveBeenCalledWith('proxmox.settings');
    expect(actions.startAdd).toHaveBeenCalled();
  });

  it('words mentions, tool failures, start failure, and turn-off question', () => {
    const context = new TestPluginContext('proxmox', true);
    plugin.register(context);
    const [mentionKind] = context.shell.mentionKinds.entries;
    const vncTool = context.shell.toolKinds.entries.find((entry) => entry.id === 'proxmox_vnc_screenshot');

    expect(mentionKind.variantName('server')).not.toBeNull();
    expect(mentionKind.variantName('vm')).not.toBeNull();
    expect(mentionKind.variantName('other')).toBeNull();
    expect(mentionKind.warning('not_ready')).not.toBeNull();
    expect(mentionKind.refusal('vm_not_allowed')).not.toBeNull();
    expect(mentionKind.summary({ code: 'vm', values: { vmid: 100 } })).toContain('100');

    expect(vncTool?.failure('owner_in_control')).not.toBeNull();
    expect(vncTool?.failure('target_changed')).not.toBeNull();
    expect(vncTool?.failure('unknown_code')).toBeNull();
    expect(vncTool?.summary({ arguments: {}, data: null, failure: null, status: 'completed' })).not.toBeNull();

    expect(plugin.failure?.('storage_failed')).not.toBeNull();
    expect(plugin.failure?.('other')).toBeNull();
    expect(plugin.offQuestion?.([{ code: 'vnc_sessions', count: 2 }])).toContain('2');
    expect(plugin.offQuestion?.([])).toBeNull();
  });
});
