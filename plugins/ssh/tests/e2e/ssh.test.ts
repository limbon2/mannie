// SSH hosts in the app (plugins/ssh/docs/README.md, acceptance), with an SSH
// server on this computer: the Plugins page lists what the plugin may use,
// with its keyed secret family, its crate and its sockets; adding a host
// opens its key dialog, which shows the server's fingerprint, and trusting
// the key tests the login; a command on the host asks the owner with the
// host, its address and its key, and runs once allowed; and removing a host
// asks first. Every step passes axe in both themes. The fixture provider's
// tools scenario calls the tool that a message names. The plugin is off in
// a new installation, so the spec turns it on first.
import {
  addProfile,
  callMessage,
  clickInLatestTurn,
  LATEST_MAIN_TURN,
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
  waitForShownText,
} from '#e2e/page.ts';
import { MODELS, type ProviderServer, startProviderServer } from '#e2e/provider-server.ts';

import { BUILD_AND_START_TIMEOUT_MILLIS, TestSshServer } from './support/server.ts';

/** The SSH plugin's row on the Plugins page. */
const PLUGIN = '[data-plugin="ssh"]';

/** The name that the owner gives the test server. */
const HOST = 'e2e-box';

/** The host's row in the SSH settings. */
const ROW = `[data-ssh-host="${HOST}"]`;

/** Runs axe in the light and the dark theme. */
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

/** Types into a field of the open dialog, after clearing it. */
async function fill(selector: string, text: string): Promise<void> {
  const field = $(selector);
  await field.click();
  await field.clearValue();
  await field.addValue(text);
}

describe('SSH hosts', () => {
  let server: TestSshServer;
  let provider: ProviderServer;

  before(async function () {
    this.timeout(BUILD_AND_START_TIMEOUT_MILLIS);
    server = await TestSshServer.start();
    provider = await startProviderServer();
    await waitForApp();
  });

  after(async () => {
    await server.stop();
    // The profiles' credentials live in the system's credential store.
    await deleteProviderProfiles();
    await provider.close();
  });

  it('lists SSH off in a new installation, with its keyed secrets, its crate and its sockets', async () => {
    await openSettings('plugins.settings');
    await expect($(PLUGIN)).toHaveAttribute('data-plugin-state', 'off');

    await openPluginUses('ssh');

    await expect($(`${PLUGIN} [data-plugin-service="secrets"] [data-plugin-service-sensitive]`)).toBeDisplayed();
    // WebdriverIO reads a selector that holds `<key>` as a tag name, so the
    // family is found by its start and its text is checked apart.
    const family = $(`${PLUGIN} [data-plugin-secret^="ssh/host/"]`);
    await expect(family).toBeDisplayed();
    await waitForShownText(family, 'ssh/host/<key>');
    await expect($(`${PLUGIN} [data-plugin-command="add_host"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-channel="follow_hosts"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-crate="russh"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-native="sockets"]`)).toBeDisplayed();
    await setPluginEnabled('ssh', true);
    await leaveSettings();
  });

  it('adds a host, shows the key that it shows, and tests the login once the key is trusted', async () => {
    await openSettings('ssh.settings');
    await $('[data-ssh-add]').click();
    await fill('#ssh-host-name', HOST);
    await fill('#ssh-host-address', '127.0.0.1');
    await fill('#ssh-host-port', String(server.port));
    await fill('#ssh-host-username', server.username);
    await fill('#ssh-host-password', server.password);
    await fill('#ssh-host-note', 'The end-to-end tests’ server.');
    await $('[data-ssh-host-submit]').click();

    await waitForShownText($('[data-ssh-fingerprint]'), server.fingerprint);
    await axeInBothThemes();
    await $('[data-ssh-trust]').click();
    await $('[data-ssh-key-tested]').waitForDisplayed({ timeout: 30_000 });
    await browser.keys('Escape');

    await expect($(`${ROW} [data-ssh-status="ready"]`)).toBeDisplayed();
    await axeInBothThemes();
    await leaveSettings();
  });

  it('runs a command on the host once the owner allows it, after showing the host and its key', async () => {
    await addProfile(provider.baseUrl, { model: MODELS.tools, name: 'Tools', useTools: true });
    await startNewConversation();

    await send(callMessage('run_ssh_command', { command: 'echo hello from e2e', host: HOST, timeout_seconds: null }));
    await waitForToolStatus('awaiting_approval');

    // The call's closed details keep a hidden copy of the list, so the shown
    // one is the approval's.
    const shown = await $$(`${LATEST_MAIN_TURN} [data-remote-command]`).filter((list) => list.isDisplayed());
    expect(shown).toHaveLength(1);
    for (const access of shown) {
      await waitForShownText(access, expect.stringContaining(HOST));
      await waitForShownText(access, expect.stringContaining(`${server.username}@127.0.0.1:${String(server.port)}`));
      await waitForShownText(access, expect.stringContaining(server.fingerprint));
    }
    await axeInBothThemes();
    await clickInLatestTurn('Allow once');

    expect(await waitForCompleteReply()).toContain('hello from e2e');
    await waitForToolStatus('completed');
  });

  it('removes a host after asking', async () => {
    await openSettings('ssh.settings');
    await $(`${ROW} [data-ssh-remove]`).click();
    await axeInBothThemes();
    await $('[data-ssh-remove-confirm]').click();

    await expect($(ROW)).not.toBeExisting();
    await leaveSettings();
  });
});
