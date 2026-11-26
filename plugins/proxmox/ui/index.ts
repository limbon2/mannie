// Frontend entry point of the Proxmox VMs and VNC plugin (ADR-0080):
// registers Settings management with fail-closed VM allowlist pre-configure
// and TLS leaf pinning dialogs, live VNC side panel, conversation menu
// selector, `@` mentions for servers and allowed VMs, and structured tool
// cards for all 11 Proxmox tools.
import { definePlugin } from '@mannie/plugin-sdk';
import MonitorIcon from '@mannie/plugin-sdk/icons/monitor';
import ServerIcon from '@mannie/plugin-sdk/icons/server';
import { openSettings } from '@mannie/plugin-sdk/navigation';
import type { Message } from '@mannie/plugin-sdk/slots';

import ProxmoxControl from './components/ProxmoxControl.svelte';
import ProxmoxDialogs from './components/ProxmoxDialogs.svelte';
import ProxmoxSettingsSection from './components/ProxmoxSettingsSection.svelte';
import ProxmoxToolDetails from './components/ProxmoxToolDetails.svelte';
import ProxmoxVncPanel from './components/ProxmoxVncPanel.svelte';
import { m } from './generated/messages.js';
import { proxmoxActions, SETTINGS_SECTION } from './state/proxmox-actions.svelte.js';
import { serverList } from './state/servers.svelte.js';
import {
  proxmoxMentionRefusal,
  proxmoxMentionSummary,
  proxmoxMentionVariantName,
  proxmoxMentionWarning,
  proxmoxOffQuestion,
  proxmoxStartFailureMessage,
  proxmoxToolFailure,
} from './utils/proxmox-messages.js';
import { PROXMOX_VNC_PANEL } from './utils/proxmox-panel.js';

interface ProxmoxToolRegistration {
  readonly icon: typeof MonitorIcon;
  readonly id: string;
  readonly label: Message;
}

const PROXMOX_TOOLS: readonly ProxmoxToolRegistration[] = [
  { icon: ServerIcon, id: 'proxmox_list_vms', label: m.proxmox_tool_list_vms },
  { icon: ServerIcon, id: 'proxmox_get_vm_config', label: m.proxmox_tool_get_vm_config },
  { icon: ServerIcon, id: 'proxmox_list_isos', label: m.proxmox_tool_list_isos },
  { icon: ServerIcon, id: 'proxmox_attach_iso', label: m.proxmox_tool_attach_iso },
  { icon: ServerIcon, id: 'proxmox_eject_iso', label: m.proxmox_tool_eject_iso },
  { icon: ServerIcon, id: 'proxmox_set_boot_order', label: m.proxmox_tool_set_boot_order },
  { icon: ServerIcon, id: 'proxmox_vm_power', label: m.proxmox_tool_vm_power },
  { icon: MonitorIcon, id: 'proxmox_vnc_screenshot', label: m.proxmox_tool_vnc_screenshot },
  { icon: MonitorIcon, id: 'proxmox_vnc_pointer', label: m.proxmox_tool_vnc_pointer },
  { icon: MonitorIcon, id: 'proxmox_vnc_keyboard', label: m.proxmox_tool_vnc_keyboard },
  { icon: MonitorIcon, id: 'proxmox_vnc_close', label: m.proxmox_tool_vnc_close },
];

export default definePlugin({
  failure: proxmoxStartFailureMessage,
  offQuestion: proxmoxOffQuestion,
  register(context) {
    const { slots } = context;
    serverList.follow(context);

    slots.overlays.register({ component: ProxmoxDialogs, id: 'proxmox.dialogs' });

    slots.commands.register({
      group: m.proxmox_command_group,
      id: 'proxmox.add-server',
      label: m.proxmox_command_add_server,
      order: 0,
      run: () => {
        void openSettings(SETTINGS_SECTION).then(() => {
          proxmoxActions.startAdd();
        });
      },
    });

    slots.settingsSections.register({
      component: ProxmoxSettingsSection,
      description: m.proxmox_settings_description,
      icon: ServerIcon,
      id: SETTINGS_SECTION,
      order: 35,
      title: m.proxmox_settings_title,
    });

    slots.sidePanels.register({
      component: ProxmoxVncPanel,
      defaultLabel: m.proxmox_panel_title,
      hideHeading: true,
      icon: MonitorIcon,
      id: PROXMOX_VNC_PANEL,
    });

    slots.conversationControls.register({
      component: ProxmoxControl,
      id: 'proxmox.control',
      order: 46,
      place: 'menu',
    });

    slots.mentionKinds.register({
      group: m.proxmox_mention_group,
      icon: (variant) => (variant === 'vm' ? MonitorIcon : ServerIcon),
      id: context.id,
      order: 25,
      refusal: proxmoxMentionRefusal,
      summary: proxmoxMentionSummary,
      variantName: proxmoxMentionVariantName,
      warning: proxmoxMentionWarning,
    });

    for (const tool of PROXMOX_TOOLS) {
      slots.toolKinds.register({
        details: ProxmoxToolDetails,
        failure: proxmoxToolFailure,
        icon: tool.icon,
        id: tool.id,
        summary: () => tool.label(),
      });
    }
  },
});
