// Side panel addressing and opening helpers for live Proxmox VNC consoles.
import {
  openConversationId,
  type OpenPanelOptions,
  openSidePanel,
  setSidePanelLabel,
} from '@mannie/plugin-sdk/navigation';

/** Registered side-panel kind ID for live Proxmox VNC sessions. */
export const PROXMOX_VNC_PANEL = 'proxmox.vnc';

/** Parsed live VNC side-panel argument. */
export interface ProxmoxVncTarget {
  readonly conversationId: string;
  readonly serverId: string;
  readonly vmid: number;
}

/** Encodes a live VNC panel argument string. */
export function formatVncPanelArgument(target: ProxmoxVncTarget): string {
  return `${target.conversationId}::${target.serverId}::${String(target.vmid)}`;
}

/** Parses a live VNC panel argument string, or returns `null` when malformed. */
export function parseVncPanelArgument(argument: string): ProxmoxVncTarget | null {
  const parts = argument.split('::');
  if (parts.length !== 3) {
    return null;
  }

  const [conversationId = '', serverId = '', vmidText = ''] = parts;
  const vmid = Number.parseInt(vmidText, 10);
  const hasIds = conversationId !== '' && serverId !== '';
  const hasValidVmid = Number.isInteger(vmid) && vmid > 0;
  if (!hasIds || !hasValidVmid) {
    return null;
  }

  return { conversationId, serverId, vmid };
}

/** Updates the tab label of an open Proxmox VNC side panel. */
export function labelVncPanel(target: ProxmoxVncTarget, vmName: string): void {
  const text = `${vmName} (${String(target.vmid)})`;
  setSidePanelLabel(PROXMOX_VNC_PANEL, formatVncPanelArgument(target), { isContent: true, text });
}

/** Opens the live Proxmox VNC side panel beside the active conversation. */
export async function openVncPanel(
  target: ProxmoxVncTarget,
  vmName: string,
  options: OpenPanelOptions = {},
): Promise<void> {
  if (openConversationId() !== target.conversationId) {
    return;
  }

  const text = `${vmName} (${String(target.vmid)})`;
  await openSidePanel(PROXMOX_VNC_PANEL, formatVncPanelArgument(target), {
    label: { isContent: true, text },
    ...options,
  });
}
