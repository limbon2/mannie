// MCP servers in the app (plugins/mcp/docs/README.md, acceptance), with the
// plugin's test server over stdio: the Plugins page lists what the plugin
// may use, with its sensitive service local-servers, its keyed secret
// families, its crates and its sockets; a local server with a secret
// variable connects, lists its tools and shows its log with the secret
// removed; a call of its tool asks the owner with the server, its program
// and the tool, and runs once allowed; the secret that the server returns
// reaches the transcript only as [REDACTED]; the image that a tool returns
// shows in the call's details; "@" mentions the server's resource, whose
// text the model reads; on Windows, a server that a .cmd file starts
// connects too; and removing a server asks first. Every step passes axe in
// both themes. The fixture provider's tools scenario calls the tool that a
// message names. The plugin is off in a new installation, so the spec turns
// it on first.
import { mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
  addProfile,
  callMessage,
  clickInLatestTurn,
  COMPOSER,
  LATEST_MAIN_TURN,
  latestTurnParts,
  send,
  startNewConversation,
  turnCount,
  waitForCompleteReply,
  waitForComposer,
  waitForNewTurn,
  waitForToolStatus,
} from '#e2e/chat.ts';
import { deleteProviderProfiles } from '#e2e/harness.ts';
import {
  axeViolations,
  leaveSettings,
  openPluginUses,
  openSettings,
  setPluginEnabled,
  waitForAnimations,
  waitForApp,
  waitForShownText,
} from '#e2e/page.ts';
import { MODELS, type ProviderServer, startProviderServer } from '#e2e/provider-server.ts';
import { Key } from 'webdriverio';

import { BUILD_TIMEOUT_MILLIS, buildServer, README_TEXT, SECRET_VARIABLE, serverProgram } from './support/server.ts';

/** The MCP plugin's row on the Plugins page. */
const PLUGIN = '[data-plugin="mcp"]';

/** The name that the owner gives the test server. */
const SERVER = 'E2E server';

/** The prefix of the test server's tools, which the owner types. */
const PREFIX = 'e2e';

/** The server's row in the MCP settings. */
const ROW = `[data-mcp-server="${SERVER}"]`;

/** The secret that the owner gives the server, which must never show. */
const PLANTED_SECRET = 'e2e-planted-mcp-secret-41c7';

/** How long a server has to start and list its tools. */
const CONNECT_TIMEOUT_MILLIS = 60_000;

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

/** The row of the dialog's secret variables, whose fields are a name and a value. */
const SECRET_ROW = '[data-mcp-row="mcp-server-secret-variables"]';

/** An open menu, which renders outside the row that opened it. */
const MENU = '[data-slot="dropdown-menu-content"]';

/** Opens a server's menu of actions and chooses the item that carries the attribute `item`. */
async function chooseServerAction(name: string, item: string): Promise<void> {
  await $(`[data-mcp-server="${name}"] [data-mcp-actions]`).click();
  await $(`${MENU} [${item}]`).click();
  await waitForAnimations();
}

/** Whether the element that `selector` finds shows inside the window, without scrolling the page. */
async function isInView(selector: string): Promise<boolean> {
  return $(selector).isDisplayed({ withinViewport: true });
}

/** Types into a field of the open dialog, after clearing it. */
async function fill(selector: string, text: string): Promise<void> {
  const field = $(selector);
  await field.click();
  await field.clearValue();
  await field.addValue(text);
}

/** Adds a local server with the dialog of the MCP settings, which must be open. */
async function addLocalServer(name: string, prefix: string, program: string): Promise<void> {
  await $('[data-mcp-add]').click();
  await fill('#mcp-server-name', name);
  await fill('#mcp-server-prefix', prefix);
  await fill('#mcp-server-program', program);
}

/** Waits until the server's row shows it as connected. */
async function waitUntilConnected(name: string): Promise<void> {
  await $(`[data-mcp-server="${name}"] [data-mcp-status="connected"]`).waitForDisplayed({
    timeout: CONNECT_TIMEOUT_MILLIS,
    timeoutMsg: `the server ${name} did not connect`,
  });
}

/** The text of the owner's latest message in the newest request to the model. */
function latestOwnerMessage(provider: ProviderServer): string {
  const requests = provider.requests().filter((request) => request.body !== null);
  const body = requests[requests.length - 1]?.body ?? {};
  const messages = (body.messages ?? []) as { content?: unknown; role?: string }[];
  const owner = messages.filter((message) => message.role === 'user').pop();
  return typeof owner?.content === 'string' ? owner.content : '';
}

/** Allows the latest call once, after checking that its request shows the server's tool. */
async function allowTheServersCall(tool: string): Promise<void> {
  await waitForToolStatus('awaiting_approval');
  // The call's closed details keep a hidden copy of the list, so the shown
  // one is the approval's.
  const shown = await $$(`${LATEST_MAIN_TURN} [data-external-tool]`).filter((list) => list.isDisplayed());
  expect(shown).toHaveLength(1);
  for (const access of shown) {
    await waitForShownText(access, expect.stringContaining(SERVER));
    await waitForShownText(access, expect.stringContaining('mcp-test-server'));
  }
  await waitForShownText($(LATEST_MAIN_TURN), expect.stringContaining(tool));
  await clickInLatestTurn('Allow once');
}

describe('MCP servers', () => {
  let provider: ProviderServer;
  let folder: string;

  before(async function () {
    this.timeout(BUILD_TIMEOUT_MILLIS);
    buildServer();
    folder = realpathSync.native(mkdtempSync(join(tmpdir(), 'mannie-e2e-mcp-')));
    provider = await startProviderServer();
    await waitForApp();
  });

  after(async () => {
    // The profiles' credentials live in the system's credential store.
    await deleteProviderProfiles();
    await provider.close();
    rmSync(folder, { force: true, recursive: true });
  });

  it('lists MCP off in a new installation, with its servers, its keyed secrets, its crates and its sockets', async () => {
    await openSettings('plugins.settings');
    await expect($(PLUGIN)).toHaveAttribute('data-plugin-state', 'off');

    await openPluginUses('mcp');

    await expect($(`${PLUGIN} [data-plugin-service="local-servers"] [data-plugin-service-sensitive]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-service="secrets"] [data-plugin-service-sensitive]`)).toBeDisplayed();
    // WebdriverIO reads a selector that holds `<key>` as a tag name, so the
    // families are found by their starts.
    for (const family of ['variable', 'header', 'oauth']) {
      await expect($(`${PLUGIN} [data-plugin-secret^="mcp/${family}/"]`)).toBeDisplayed();
    }
    await expect($(`${PLUGIN} [data-plugin-command="add_server"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-channel="follow_servers"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-crate="rmcp"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-crate="oauth2"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-native="sockets"]`)).toBeDisplayed();
    await setPluginEnabled('mcp', true);
    await leaveSettings();
  });

  it('adds a local server with a secret variable, which connects, lists its tools and logs without the secret', async () => {
    await openSettings('mcp.settings');
    await addLocalServer(SERVER, PREFIX, serverProgram());
    // The two kinds of server share a line, and each field shows an example.
    const stdio = await $('#mcp-transport-stdio').getLocation();
    const http = await $('#mcp-transport-http').getLocation();
    expect(Math.abs(http.y - stdio.y)).toBeLessThan(1);
    for (const field of [
      '#mcp-server-name',
      '#mcp-server-prefix',
      '#mcp-server-program',
      '#mcp-server-arguments',
      '#mcp-server-folder',
    ]) {
      expect(await $(field).getAttribute('placeholder')).toMatch(/\S/u);
    }
    await $('[data-mcp-add-row="mcp-server-secret-variables"]').click();
    await fill(`${SECRET_ROW} input:nth-of-type(1)`, SECRET_VARIABLE);
    await fill(`${SECRET_ROW} input:nth-of-type(2)`, PLANTED_SECRET);
    // Only the fields scroll, so the dialog's buttons stay in view.
    expect(await isInView('[data-mcp-server-submit]')).toBe(true);
    await axeInBothThemes();
    await $('[data-mcp-server-submit]').click();

    await waitUntilConnected(SERVER);
    await axeInBothThemes();

    await $(`${ROW} [data-mcp-actions]`).click();
    await $(`${MENU} [data-mcp-tools]`).waitForDisplayed();
    await axeInBothThemes();
    await $(`${MENU} [data-mcp-tools]`).click();
    await $('[data-mcp-tools-dialog] [data-mcp-tool="echo"]').waitForDisplayed();
    await expect($('[data-mcp-tools-dialog] [data-mcp-tool="show_image"]')).toBeDisplayed();
    expect(await isInView('[data-mcp-tools-dialog] [data-slot="dialog-footer"] button')).toBe(true);
    await axeInBothThemes();
    await browser.keys('Escape');
    await waitForAnimations();
    // The focus goes back to the menu's button, since the item that opened the dialog is gone.
    expect(await browser.execute(() => document.activeElement?.hasAttribute('data-mcp-actions') ?? false)).toBe(true);

    await chooseServerAction(SERVER, 'data-mcp-log');
    const log = $('[data-mcp-log-dialog]');
    await waitForShownText(log, expect.stringContaining('starting with the token [REDACTED]'));
    await expect(log).not.toHaveText(expect.stringContaining(PLANTED_SECRET));
    await axeInBothThemes();
    await browser.keys('Escape');
    await waitForAnimations();
    await leaveSettings();
  });

  it('calls a tool of the server once the owner allows it, after showing the server, its program and the tool', async () => {
    await addProfile(provider.baseUrl, { model: MODELS.tools, name: 'Tools', useTools: true });
    await startNewConversation();

    await send(callMessage(`mcp_${PREFIX}_echo`, { text: 'hello from e2e' }));
    await waitForToolStatus('awaiting_approval');
    await axeInBothThemes();
    await allowTheServersCall('echo');

    expect(await waitForCompleteReply()).toContain('Echo: hello from e2e');
    await waitForToolStatus('completed');
  });

  it('shows the secret that the server returns only as [REDACTED]', async () => {
    await send(callMessage(`mcp_${PREFIX}_read_variable`, { name: SECRET_VARIABLE }));
    await allowTheServersCall('read_variable');

    const reply = await waitForCompleteReply();
    expect(reply).toContain('[REDACTED]');
    expect(reply).not.toContain(PLANTED_SECRET);
    await waitForToolStatus('completed');
    await expect($('main')).not.toHaveText(expect.stringContaining(PLANTED_SECRET));
    for (const request of provider.requests()) {
      expect(JSON.stringify(request.body)).not.toContain(PLANTED_SECRET);
    }
  });

  it('shows the image that a tool returns in the call’s details', async () => {
    await send(callMessage(`mcp_${PREFIX}_show_image`, {}));
    await allowTheServersCall('show_image');
    await waitForCompleteReply();
    await waitForToolStatus('completed');

    await $(`${LATEST_MAIN_TURN} [data-tool-toggle]`).click();
    await waitForAnimations();
    const images = $(`${LATEST_MAIN_TURN} [data-tool-images]`);
    await images.waitForDisplayed();
    await images.$('button').click();
    await $('[role="dialog"] img').waitForDisplayed();
    await axeInBothThemes();
    await browser.keys('Escape');
    await waitForAnimations();
  });

  it('mentions a resource of the server with "@", whose text the model reads', async () => {
    await waitForComposer();
    await $(COMPOSER).click();
    await browser.keys('Summarize @Read');
    await browser.waitUntil(
      async () =>
        browser.execute(() =>
          [...document.querySelectorAll('[data-mention-list] [role="option"] [data-content]')].some(
            (label) => label.textContent.trim() === 'Read me',
          ),
        ),
      { timeout: 10_000, timeoutMsg: 'the list did not show the resource' },
    );
    await axeInBothThemes();
    await browser.keys(Key.Enter);
    const before = await turnCount();
    await browser.keys(Key.Enter);
    await waitForNewTurn(before);
    await waitForCompleteReply();

    const kinds = (await latestTurnParts()).map((part) => part.kind);
    expect(kinds.slice(0, 2)).toEqual(['user_text', 'mention']);
    expect(latestOwnerMessage(provider)).toContain(README_TEXT);
  });

  it('connects a server that a .cmd file starts, on Windows', async function () {
    if (process.platform !== 'win32') {
      this.skip();
    }
    const wrapper = join(folder, 'mcp-wrapper.cmd');
    writeFileSync(wrapper, `@echo off\r\n"${serverProgram()}" %*\r\n`);
    await openSettings('mcp.settings');
    await addLocalServer('Wrapped', 'wrap', wrapper);
    await $('[data-mcp-server-submit]').click();

    await waitUntilConnected('Wrapped');
    await chooseServerAction('Wrapped', 'data-mcp-remove');
    await $('[data-mcp-remove-confirm]').click();
    await expect($('[data-mcp-server="Wrapped"]')).not.toBeExisting();
    await leaveSettings();
  });

  it('removes a server after asking', async () => {
    await openSettings('mcp.settings');
    await chooseServerAction(SERVER, 'data-mcp-remove');
    await axeInBothThemes();
    await $('[data-mcp-remove-confirm]').click();

    await expect($(ROW)).not.toBeExisting();
    await leaveSettings();
  });
});
