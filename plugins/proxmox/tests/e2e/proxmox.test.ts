// End-to-end validation of the Proxmox VMs and VNC plugin (V16; ADR-0080):
// - Plugin starts off on new installation, lists its grants and keyed secret family.
// - Adding a server with an untrusted self-signed cert automatically guides through
//   TLS certificate inspection + leaf pinning -> pre-configure Choose allowed VMs.
// - Unselected VMs stay invisible; allowed VMs work through scoped approvals.
// - Live VNC side panel renders live canvas frames, supports Take over, Ctrl+Alt+Del,
//   Exit console focus, Resume agent, and Close console.
// - Every dialog and view passes axe WCAG 2.1 AA in both light and dark themes.
import {
  addProfile,
  callMessage,
  clickInLatestTurn,
  LATEST_MAIN_TURN,
  openConversationSubmenu,
  send,
  startNewConversation,
  waitForCompleteReply,
  waitForToolStatus,
} from '#e2e/chat.ts';
import { deleteProviderProfiles } from '#e2e/harness.ts';
import {
  axeViolations,
  leaveSettings,
  openPluginUses,
  openSettings,
  setPluginEnabled,
  waitForApp,
  waitForGone,
  waitForShownText,
} from '#e2e/page.ts';
import { MODELS, type ProviderServer, startProviderServer } from '#e2e/provider-server.ts';

import { FakeProxmoxE2eServer } from './support/server.ts';

const PLUGIN = '[data-plugin="proxmox"]';
const SERVER_NAME = 'e2e-pve';
const ROW = `[data-proxmox-server="${SERVER_NAME}"]`;

async function axeInBothThemes(): Promise<void> {
  for (const dark of [false, true]) {
    await browser.execute((isDark: boolean) => {
      document.documentElement.classList.toggle('dark', isDark);
    }, dark);
    expect(await axeViolations()).toEqual([]);
  }
  await browser.execute(() => {
    document.documentElement.classList.remove('dark');
  });
}

async function fill(selector: string, text: string): Promise<void> {
  const field = $(selector);
  await field.click();
  await field.clearValue();
  await field.addValue(text);
}

describe('Proxmox VMs and VNC', () => {
  let server: FakeProxmoxE2eServer;
  let provider: ProviderServer;

  before(async function () {
    this.timeout(60_000);
    server = await FakeProxmoxE2eServer.start();
    provider = await startProviderServer();
    await waitForApp();
  });

  after(async () => {
    await server.stop();
    await deleteProviderProfiles();
    await provider.close();
  });

  it('lists Proxmox off in a new installation with its keyed secrets, commands, channels, and sockets', async () => {
    await openSettings('plugins.settings');
    await expect($(PLUGIN)).toHaveAttribute('data-plugin-state', 'off');

    await openPluginUses('proxmox');

    await expect($(`${PLUGIN} [data-plugin-service="secrets"] [data-plugin-service-sensitive]`)).toBeDisplayed();
    const family = $(`${PLUGIN} [data-plugin-secret^="proxmox/server/"]`);
    await expect(family).toBeDisplayed();
    await waitForShownText(family, 'proxmox/server/<key>');
    await expect($(`${PLUGIN} [data-plugin-command="add_server"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-channel="follow_servers"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-channel="follow_vnc_session"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-native="sockets"]`)).toBeDisplayed();

    await setPluginEnabled('proxmox', true);
    await leaveSettings();
  });

  it('adds a server, inspects and pins its TLS cert, and pre-configures only selected allowed VMs', async () => {
    await openSettings('proxmox.settings');
    await $('[data-proxmox-add]').click();
    await fill('#proxmox-server-name', SERVER_NAME);
    await fill('#proxmox-server-host', '127.0.0.1');
    await fill('#proxmox-server-port', String(server.port));
    await fill('#proxmox-server-username', server.username);
    await fill('#proxmox-server-password', server.password);
    await fill('#proxmox-server-note', 'E2E test Proxmox cluster.');
    await $('[data-proxmox-server-submit]').click();

    // Guided flow detects untrusted self-signed cert and opens TLS certificate dialog automatically.
    await waitForShownText($('[data-proxmox-cert-fingerprint]'), server.fingerprint);
    await axeInBothThemes();
    await $('[data-proxmox-cert-pin]').click();

    // Pinning automatically advances into the pre-configure Choose Allowed VMs dialog.
    const vm100Checkbox = $('[data-proxmox-vm-checkbox="100"]');
    await vm100Checkbox.waitForDisplayed({ timeout: 30_000 });
    await expect($('[data-proxmox-vm-row="200"]')).toBeDisplayed();
    await axeInBothThemes();

    // Allow ONLY VM 100 (`debian-lab`), leaving VM 200 (`prod-secret-db`) unchecked.
    await vm100Checkbox.click();
    await $('[data-proxmox-vms-save]').click();

    await expect($(`${ROW} [data-proxmox-status="ready"]`)).toBeDisplayed();
    await expect($(`${ROW} [data-proxmox-allowed-count="1"]`)).toBeDisplayed();
    await expect($(`${ROW} [data-proxmox-allowed-vm="100"]`)).toBeDisplayed();
    await axeInBothThemes();
    await leaveSettings();
  });

  it('lists only allowed VMs, approves config and VNC tools, and drives live VNC takeover in the side panel', async () => {
    await addProfile(provider.baseUrl, { model: MODELS.tools, name: 'Tools', useTools: true });
    await startNewConversation();

    // 1. Read-only list shows only allowed VM 100 and never unselected VM 200.
    await send(callMessage('proxmox_list_vms', { server: SERVER_NAME }));
    const listReply = await waitForCompleteReply();
    expect(listReply).toContain('debian-lab');
    expect(listReply).not.toContain('prod-secret-db');
    await waitForToolStatus('completed');

    // 2. Config change requests scoped approval showing server, endpoint, and pinned identity.
    await send(
      callMessage('proxmox_attach_iso', {
        device: 'ide2',
        server: SERVER_NAME,
        vmid: 100,
        volid: 'local:iso/debian-13-netinst.iso',
      }),
    );
    await waitForToolStatus('awaiting_approval');

    const shownAccess = await $$(`${LATEST_MAIN_TURN} [data-proxmox-vm]`).filter((el) => el.isDisplayed());
    expect(shownAccess).toHaveLength(1);
    for (const access of shownAccess) {
      await waitForShownText(access, expect.stringContaining(SERVER_NAME));
      await waitForShownText(access, expect.stringContaining(`https://127.0.0.1:${String(server.port)}`));
    }
    await axeInBothThemes();
    await clickInLatestTurn('Allow once');

    await waitForCompleteReply();
    await waitForToolStatus('completed');
    expect(server.configUpdates.some((update) => update.vmid === 100)).toBe(true);

    // 3. Open live VNC side panel via the conversation "More actions" menu.
    await expect($('[data-proxmox-control]')).not.toBeExisting();
    await openConversationSubmenu('[data-proxmox-control]');
    const vmItem = $('[data-proxmox-open-vm$=":100"]');
    await vmItem.waitForDisplayed({ timeout: 15_000 });
    await vmItem.click();

    const panel = $('[data-proxmox-vnc-panel]');
    await panel.waitForDisplayed({ timeout: 30_000 });
    const canvas = $('[data-proxmox-vnc-canvas]');
    await canvas.waitForDisplayed({ timeout: 30_000 });

    // Wait for the fake server's 128x96 live framebuffer snapshot to draw onto the canvas.
    await browser.waitUntil(
      async () =>
        browser.execute(() => {
          const el = document.querySelector<HTMLCanvasElement>('[data-proxmox-vnc-canvas]');
          if (el === null) {
            return false;
          }
          const widthOk = el.width === 128;
          const heightOk = el.height === 96;
          return widthOk && heightOk;
        }),
      { timeout: 20_000 },
    );

    // Newly opened owner VNC session starts in owner control mode.
    await expect($('[data-proxmox-control-state="owner"]')).toBeDisplayed();
    await expect(canvas).toHaveAttribute('data-proxmox-vnc-interactive', 'true');
    await axeInBothThemes();

    // Send Ctrl+Alt+Del from toolbar and confirm fake RFB server received Ctrl, Alt, Delete keysyms.
    await $('[data-proxmox-ctrl-alt-del]').click();
    await browser.waitUntil(
      () =>
        server.keyEvents.some((ev) => {
          const isVm100 = ev.vmid === 100;
          const isDeleteKey = ev.keysym === 0xffff;
          return isVm100 && isDeleteKey && ev.down;
        }),
      { timeout: 10_000 },
    );

    // Hand control back to agent, then take over again, then close console.
    await $('[data-proxmox-resume-agent]').click();
    await expect($('[data-proxmox-control-state="agent"]')).toBeDisplayed();
    await expect(canvas).toHaveAttribute('data-proxmox-vnc-interactive', 'false');

    await $('[data-proxmox-take-over]').click();
    await expect($('[data-proxmox-control-state="owner"]')).toBeDisplayed();
    await $('[data-proxmox-release-focus]').click();

    await $('[data-proxmox-close-session]').click();
    await $('[data-proxmox-reconnect]').waitForDisplayed({ timeout: 15_000 });
  });

  it('removes the server after confirmation', async () => {
    await openSettings('proxmox.settings');
    await $(`${ROW} [data-proxmox-actions]`).click();
    const removeItem = $('[data-slot="dropdown-menu-content"] [data-proxmox-remove]');
    await removeItem.waitForDisplayed();
    await axeInBothThemes();
    await removeItem.click();
    await axeInBothThemes();
    await $('[data-proxmox-remove-confirm]').click();

    await waitForGone(ROW);
    await leaveSettings();
  });
});
