import { randomUUID } from 'node:crypto';
import { existsSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync } from 'node:fs';
import { createServer, type Server } from 'node:http';
import { tmpdir } from 'node:os';
import { basename, dirname, join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

import { dataDirectory } from '#e2e/app.ts';
import {
  addProfile,
  callMessage,
  clickInLatestTurn,
  clickWithText,
  LATEST_MAIN_TURN,
  MAIN_TURNS,
  send,
  sendLines,
  sendMessage,
  startNewConversation,
  waitForCompleteReply,
  waitForToolStatus,
} from '#e2e/chat.ts';
import {
  browserEvaluate,
  type BrowserSurface,
  browserSurfaces,
  chooseTrayItem,
  deleteProviderProfiles,
  processId,
  setFolderChoice,
} from '#e2e/harness.ts';
import {
  axeViolations,
  isDark,
  leaveSettings,
  openSettings,
  runCommand,
  setPluginEnabled,
  shownText,
  waitForApp,
  waitForShownText,
} from '#e2e/page.ts';
import { MODELS, type ProviderServer, startProviderServer, TOOLS_DONE } from '#e2e/provider-server.ts';
import { Key } from 'webdriverio';

import { fixtureCertificateInWindowsTrust, selfSignedCertificate, startHttpsSite } from './support/https-site.ts';

const PANEL = '[data-side-panels] [role="tabpanel"][data-state="active"] [data-browser-panel]';
const VIEWPORT = `${PANEL} [data-browser-viewport]`;
const STILL = `${VIEWPORT} img`;
const ACTIVITY = `${PANEL} [data-browser-activity]`;
const SHELL_TABS = '[data-side-panels] [role="tablist"] [role="tab"]';
const CERTIFICATE_ERROR = `${PANEL} [data-browser-certificate-error]`;
const BROWSER_SETTINGS = '[data-browser-settings]';
const CERTIFICATES = `${BROWSER_SETTINGS} [data-browser-trusted-hosts]`;
const CREDENTIALS = `${BROWSER_SETTINGS} [data-browser-site-credentials]`;
const CERTIFICATE_DIALOG = '[role="dialog"] [data-browser-certificate-dialog]';
const CREDENTIAL_DIALOG = '[role="dialog"] [data-browser-credential-dialog]';
const ADD_CERTIFICATE = `${CERTIFICATES} [data-browser-certificate-add]`;
const ADD_CREDENTIAL = `${CREDENTIALS} [data-browser-credential-add]`;
const OWNER_ADDRESS = `${PANEL} [aria-label="Address"]:not([readonly])`;
const CREDENTIAL_APPROVAL = `${LATEST_MAIN_TURN} [data-tool-status="awaiting_approval"] > :not([data-slot="collapsible"]) [data-browser-credential-access]`;

const form = `<!doctype html>
<html lang="en">
  <head><title>Browser fixture</title></head>
  <body>
    <main>
      <h1>Browser fixture</h1>
      <form method="post" action="/submitted">
        <label>City <input name="city"></label>
        <label>Note <input name="note"></label>
        <button type="submit">Submit form</button>
      </form>
    </main>
  </body>
</html>`;

const cursorPage = `<!doctype html>
<html lang="en">
  <head>
    <title>Browser fixture</title>
    <style>button { position: absolute; width: 120px; height: 40px; box-sizing: border-box; }</style>
  </head>
  <body>
    <main>
      <h1>Browser fixture</h1>
      <p id="cursor-feedback"></p>
      <button style="left: 80px; top: 96px" onclick="document.getElementById('cursor-feedback').textContent = 'First pressed'">
        First target
      </button>
      <button style="right: 8%; top: 220px" onclick="document.getElementById('cursor-feedback').textContent = 'Second pressed'">
        Second target
      </button>
    </main>
  </body>
</html>`;

const todoNames = ['Pack the bag', 'Water the plants', 'Send the invoice'];
const todoPage = `<!doctype html>
<html lang="en">
  <head><title>Batch todos</title></head>
  <body>
    <main>
      <h1>Batch todos</h1>
      <p id="todo-status"></p>
      <ul id="todos"></ul>
    </main>
    <script>
      const todos = ${JSON.stringify(todoNames)}.map((text, index) => ({ text, done: index === 0 }));
      function render() {
        const rows = todos.map(todo => {
          const row = document.createElement('li');
          const check = document.createElement('input');
          check.type = 'checkbox';
          check.checked = todo.done;
          check.setAttribute('aria-label', 'Mark ' + todo.text + ' complete');
          check.addEventListener('change', () => {
            todo.done = check.checked;
            render();
          });
          row.append(check, todo.text);
          return row;
        });
        document.getElementById('todos').replaceChildren(...rows);
        document.getElementById('todo-status').textContent = todos.filter(todo => todo.done).length + ' of 3 complete';
      }
      render();
    </script>
  </body>
</html>`;

const signInForm = `<!doctype html>
<html lang="en">
  <head><title>Fixture sign-in</title></head>
  <body>
    <main>
      <h1>Fixture sign-in</h1>
      <form method="post" action="/sign-in/complete">
        <label for="password">Password</label>
        <input id="password" name="password" type="password"
          oninput="document.getElementById('feedback').textContent = this.value">
        <button type="submit">Sign in</button>
      </form>
      <p id="feedback"></p>
    </main>
  </body>
</html>`;

const fixturePages = new Map([
  ['/agent-approved', cursorPage],
  ['/sign-in', signInForm],
  ['/todos', todoPage],
]);

interface Site {
  address: string;
  requests: string[];
  server: Server;
  signIns: string[];
  submissions: string[];
}

/** A real page, with a POST target observed independently of the browser. */
async function startSite(): Promise<Site> {
  const requests: string[] = [];
  const signIns: string[] = [];
  const submissions: string[] = [];
  const server = createServer((request, response) => {
    requests.push(request.url ?? '');
    response.setHeader('Content-Type', 'text/html; charset=utf-8');
    const submits = request.method === 'POST' && request.url === '/submitted';
    const signsIn = request.method === 'POST' && request.url === '/sign-in/complete';
    if (submits || signsIn) {
      const chunks: Buffer[] = [];
      request.on('data', (chunk: Buffer) => chunks.push(chunk));
      request.on('end', () => {
        const body = Buffer.concat(chunks).toString('utf8');
        if (signsIn) {
          signIns.push(body);
          const value = new URLSearchParams(body).get('password') ?? '';
          const escaped = value.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
          response.end(
            `<!doctype html><html lang="en"><title>Signed in</title><h1>Signed in</h1><p>Received ${escaped}</p></html>`,
          );
          return;
        }
        submissions.push(body);
        response.end('<!doctype html><html lang="en"><title>Form submitted</title><h1>Form submitted</h1></html>');
      });
      return;
    }
    response.end(fixturePages.get(request.url ?? '') ?? form);
  });
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const address = server.address();
  const hasPort = address !== null && typeof address !== 'string';
  if (!hasPort) {
    throw new Error('browser fixture did not open a TCP address');
  }
  return { address: `http://127.0.0.1:${address.port}`, requests, server, signIns, submissions };
}

function closeSite(server: Server): Promise<void> {
  server.closeAllConnections();
  return new Promise<void>((resolve, reject) => {
    server.close((error) => {
      if (error === undefined) {
        resolve();
      } else {
        reject(error);
      }
    });
  });
}

function isRecord(value: unknown): value is Record<string, unknown> {
  const object = typeof value === 'object' && value !== null;
  return object;
}

/** The scripted provider repeats the last tool result verbatim. */
function observation(reply: string): Record<string, unknown> {
  const prefix = `${TOOLS_DONE} Last result: `;
  if (!reply.startsWith(prefix)) {
    throw new Error('the fixture provider returned no browser result');
  }
  const data: unknown = JSON.parse(reply.slice(prefix.length));
  if (!isRecord(data)) {
    throw new Error('the browser result was not a structured observation');
  }
  return data;
}

function textField(data: Record<string, unknown>, field: string): string {
  const value = data[field];
  if (typeof value !== 'string') {
    throw new Error(`missing the browser observation field ${field}`);
  }
  return value;
}

function namedEntry(list: unknown, field: 'label' | 'name', label: string): Record<string, unknown> {
  if (!Array.isArray(list)) {
    throw new Error(`the browser observation has no ${field} list`);
  }
  for (const entry of list as readonly unknown[]) {
    if (!isRecord(entry)) {
      continue;
    }
    if (entry[field] === label) {
      return entry;
    }
  }
  throw new Error(`the browser observation did not include ${label}`);
}

function reference(list: unknown, field: 'label' | 'name', label: string): string {
  return textField(namedEntry(list, field, label), 'reference');
}

async function allowBrowserSite(): Promise<string> {
  await waitForToolStatus('awaiting_approval');
  await clickInLatestTurn('Allow for this conversation');
  await waitForToolStatus('completed');
  return waitForCompleteReply();
}

async function clickBrowserTarget(
  page: Record<string, unknown>,
  name: string,
  feedback: string,
  checkAccent = false,
): Promise<string> {
  await startCursorSample(await cursorTarget(name), checkAccent);
  await sendLines([
    callMessage('browser_interact', {
      action: { kind: 'click', target: { kind: 'element', reference: reference(page.outline, 'name', name) } },
      revision: textField(page, 'revision'),
    }),
    'next',
    callMessage('browser_observe', { screenshot: false }),
  ]);
  const reply = await waitForCompleteReply();
  expect(reply).toContain(feedback);
  await expectCursorSample(name);
  return reply;
}

/** The conversation and tab that the visible Browser panel shows. */
async function shownTab(): Promise<{ conversationId: string; tabId: string }> {
  const panel = $(PANEL);
  const conversationId = await panel.getAttribute('data-browser-conversation');
  const tabId = await panel.getAttribute('data-browser-tab');
  if (conversationId === null) {
    throw new Error('the visible Browser panel does not address a conversation');
  }
  if (tabId === null) {
    throw new Error('the visible Browser panel does not address a tab');
  }
  return { conversationId, tabId };
}

/** Waits until the shown tab's page window meets `accept`, and returns it. */
async function waitForSurface(
  accept: (surface: BrowserSurface, placeholder: DOMRect, scale: number) => boolean,
  message: string,
): Promise<BrowserSurface> {
  const { conversationId, tabId } = await shownTab();
  let last: BrowserSurface | undefined;
  try {
    await browser.waitUntil(
      async () => {
        const [placeholder, scale] = await browser.execute((selector) => {
          const element = document.querySelector(selector);
          return [element?.getBoundingClientRect().toJSON() as DOMRect, window.devicePixelRatio] as const;
        }, VIEWPORT);
        last = (await browserSurfaces(conversationId)).find((surface) => surface.tab === tabId);
        return last === undefined ? false : accept(last, placeholder, scale);
      },
      { timeout: 15_000, timeoutMsg: message },
    );
  } catch (cause) {
    throw new Error(`${message}: ${JSON.stringify(last)}`, { cause });
  }
  if (last === undefined) {
    throw new Error(message);
  }
  return last;
}

/** Whether the page window covers its placeholder, within a pixel of rounding. */
function coversPlaceholder(surface: BrowserSurface, placeholder: DOMRect, scale: number): boolean {
  const near = (actual: number, expected: number) => Math.abs(actual - expected) <= 2;
  const placed = near(surface.x, placeholder.x * scale) && near(surface.y, placeholder.y * scale);
  const sized = near(surface.width, placeholder.width * scale) && near(surface.height, placeholder.height * scale);
  return surface.visible && placed && sized;
}

async function cursorTarget(name: string): Promise<[number, number]> {
  const target = await pageValue(
    `(() => {
      const button = [...document.querySelectorAll("button")].find(({ textContent }) => textContent.trim() === ${JSON.stringify(name)});
      const bounds = button.getBoundingClientRect();
      return [Math.floor(bounds.x + bounds.width / 2) * devicePixelRatio, Math.floor(bounds.y + bounds.height / 2) * devicePixelRatio];
    })()`,
  );
  expect(target).toEqual([expect.any(Number), expect.any(Number)]);
  return target as [number, number];
}

declare global {
  interface Window {
    browserCursorSample?: Promise<string | null>;
  }
}

/** Starts before the click so the webview can sample and recolor the cursor without WebDriver delays. */
async function startCursorSample(target: readonly [number, number], checkAccent: boolean): Promise<void> {
  const { conversationId, tabId } = await shownTab();
  await browser.execute(
    (conversation, tab, [x, y], checkColor) => {
      const harness = window.mannieValidation;
      if (harness === undefined) {
        throw new Error('the validation harness is missing');
      }

      const waitForCursor = async (stage: string, accent: string | null, whileVisible = false): Promise<void> => {
        const deadline = performance.now() + 15_000;
        let surface: BrowserSurface | undefined;
        while (performance.now() < deadline) {
          surface = (await harness.browserSurface(conversation)).find((entry) => entry.tab === tab);
          const cursor = surface?.cursor;
          const isVisible = cursor !== undefined && cursor !== null;
          if (isVisible) {
            const nearX = Math.abs(cursor[0] - x) <= 2;
            const nearY = Math.abs(cursor[1] - y) <= 2;
            const matchesAccent = accent === null || JSON.stringify(surface?.cursorAccent) === accent;
            if (nearX && nearY && matchesAccent) {
              return;
            }
          } else if (whileVisible) {
            break;
          }
          await new Promise<void>((resolve) => {
            window.setTimeout(resolve, 50);
          });
        }
        const expected = accent === null ? '' : ` with RGB ${accent}`;
        throw new Error(
          `${stage}: expected a visible cursor at (${String(x)}, ${String(y)})${expected}, saw ${JSON.stringify(surface)}`,
        );
      };

      window.browserCursorSample = (async () => {
        if (!checkColor) {
          await waitForCursor('position', null);
          return null;
        }

        const root = document.documentElement;
        const wasRose = root.classList.contains('palette-primary-rose');
        const wasDark = root.classList.contains('dark');
        const canvas = document.createElement('canvas');
        canvas.width = 1;
        canvas.height = 1;
        const context = canvas.getContext('2d', { willReadFrequently: true });
        if (context === null) {
          throw new Error('could not sample the active primary color');
        }
        const readPrimary = (): string => {
          context.fillStyle = getComputedStyle(root).getPropertyValue('--primary');
          context.fillRect(0, 0, 1, 1);
          return JSON.stringify(Array.from(context.getImageData(0, 0, 1, 1).data.slice(0, 3)));
        };

        try {
          const original = readPrimary();
          await waitForCursor('initial primary', original);

          root.classList.add('palette-primary-rose');
          const rose = readPrimary();
          if (rose === original) {
            throw new Error(`selecting Rose left the CSS primary at ${rose}`);
          }
          await waitForCursor('Rose primary', rose, true);

          root.classList.toggle('dark', !wasDark);
          const otherMode = readPrimary();
          if (otherMode === rose) {
            throw new Error(`switching Light/Dark left the CSS primary at ${rose}`);
          }
          await waitForCursor('Light/Dark change', otherMode, true);
          return null;
        } finally {
          root.classList.toggle('palette-primary-rose', wasRose);
          root.classList.toggle('dark', wasDark);
        }
      })().catch((error: unknown) => (error instanceof Error ? error.message : String(error)));
    },
    conversationId,
    tabId,
    target,
    checkAccent,
  );
}

async function expectCursorSample(name: string): Promise<void> {
  const failure = await browser.execute(() => window.browserCursorSample ?? 'the cursor sampler did not start');
  if (failure !== null) {
    throw new Error(`Expected the agent cursor at ${name}. ${failure}`);
  }
}

/** Reads a value in the shown tab's page. */
async function pageValue(expression: string): Promise<unknown> {
  const { conversationId, tabId } = await shownTab();
  return browserEvaluate(conversationId, tabId, expression);
}

async function openBrowserConversation(message: string): Promise<void> {
  await startNewConversation();
  await sendMessage(message);
  await waitForCompleteReply();
  await openNewBrowserTab();
}

async function openNewBrowserTab(): Promise<void> {
  await $('[data-browser-control]').click();
  const newTab = $('[data-browser-new-tab]');
  await newTab.waitForClickable();
  await newTab.click();
  await waitForShownText($(PANEL), expect.stringContaining('You have control'));
  await $(OWNER_ADDRESS).waitForEnabled();
}

async function startBrowserSession(): Promise<void> {
  await clickWithText(`${PANEL} button:not(:disabled)`, 'Start browser');
  await waitForShownText($(PANEL), expect.stringContaining('You have control'));
  await $(OWNER_ADDRESS).waitForEnabled();
}

async function endBrowserSession(): Promise<void> {
  await clickWithText(`${PANEL} button:not(:disabled)`, 'End session');
  await clickWithText('[role="alertdialog"] button', 'End session');
  await waitForShownText($(PANEL), expect.stringContaining('No browser session'));
}

async function restartMannie(): Promise<void> {
  const previousProcess = await processId();
  try {
    await chooseTrayItem('quit');
  } catch (error) {
    // The native window can close before WebDriver delivers the reply.
    const sessionEnded = error instanceof Error && error.message.includes('invalid session id');
    if (!sessionEnded) {
      throw error;
    }
  }
  const deadline = Date.now() + 15_000;
  for (;;) {
    try {
      process.kill(previousProcess, 0);
    } catch (error) {
      const processGone = error instanceof Error && 'code' in error && error.code === 'ESRCH';
      if (processGone) {
        break;
      }
      throw error;
    }
    if (Date.now() >= deadline) {
      throw new Error('Mannie did not quit');
    }
    await delay(250);
  }

  await browser.reloadSession();
  await waitForApp();
  expect(await processId()).not.toBe(previousProcess);
}

async function ownerNavigate(url: string): Promise<void> {
  const address = $(OWNER_ADDRESS);
  await address.waitForExist();
  await address.waitForEnabled();
  await address.setValue(url);
  await browser.keys(Key.Enter);
  await address.waitForExist();
}

async function openBrowserSettings(): Promise<void> {
  await openSettings('browser.settings');
  await $(BROWSER_SETTINGS).waitForDisplayed();
}

async function trustCertificate(hostname: string, fingerprint: string): Promise<void> {
  await openBrowserSettings();
  await $(ADD_CERTIFICATE).click();
  await $(CERTIFICATE_DIALOG).waitForDisplayed();
  await $(`${CERTIFICATE_DIALOG} [data-browser-certificate-hostname]`).setValue(hostname);
  await $(`${CERTIFICATE_DIALOG} [data-browser-certificate-fingerprint]`).setValue(fingerprint);
  await $(`${CERTIFICATE_DIALOG} [data-browser-certificate-save]`).click();
  await $(CERTIFICATE_DIALOG).waitForDisplayed({ reverse: true });
  const saved = $(`${CERTIFICATES} [role="listitem"][data-browser-saved-certificate]`);
  await waitForShownText(saved, expect.stringContaining(fingerprint.replaceAll(':', '').toLowerCase()));
  await expect($(ADD_CERTIFICATE)).toBeFocused();
  await leaveSettings();
}

async function forgetCertificate(fingerprint: string): Promise<void> {
  await openSettings('browser.settings');
  const normalized = fingerprint.replaceAll(':', '').toLowerCase();
  await $(CERTIFICATES).waitForDisplayed();
  await browser.waitUntil(
    async () =>
      (await $(`${CERTIFICATES} [data-browser-saved-certificate]`).isExisting()) ||
      (await shownText(await $(CERTIFICATES))).includes('No saved certificates.'),
  );
  for (const entry of await $$(`${CERTIFICATES} [data-browser-saved-certificate]`)) {
    if ((await entry.getText()).includes(normalized)) {
      await entry.$('[data-browser-certificate-forget]').click();
      await $('[role="alertdialog"] [data-browser-certificate-forget-confirm]').click();
      await entry.waitForExist({ reverse: true });
      break;
    }
  }
  await leaveSettings();
}

async function saveCredential(origin: string, label: string, value: string): Promise<void> {
  await $(ADD_CREDENTIAL).click();
  await $(CREDENTIAL_DIALOG).waitForDisplayed();
  await $(`${CREDENTIAL_DIALOG} input[name="origin"]`).setValue(origin);
  await $(`${CREDENTIAL_DIALOG} input[name="label"]`).setValue(label);
  await $(`${CREDENTIAL_DIALOG} input[name="value"]`).setValue(value);
  await $(`${CREDENTIAL_DIALOG} [data-browser-credential-save]`).click();
  await $(CREDENTIAL_DIALOG).waitForDisplayed({ reverse: true });
  const saved = $(`${CREDENTIALS} [role="listitem"]`);
  await waitForShownText(saved, expect.stringContaining(label));
  await expect($(ADD_CREDENTIAL)).toBeFocused();
  expect(await saved.getText()).not.toContain(value);
}

async function waitForHttpsFixture(path: string): Promise<void> {
  await browser.waitUntil(
    async () =>
      (await pageValue("document.documentElement.dataset.fixtureScript === 'loaded' && location.pathname")) === path,
    {
      timeoutMsg: `the HTTPS page at ${path} did not load its script`,
    },
  );
  await $(CERTIFICATE_ERROR).waitForDisplayed({ reverse: true });
}

async function waitForCertificateError(): Promise<void> {
  await $(CERTIFICATE_ERROR).waitForDisplayed();
  expect(await $(CERTIFICATES).isDisplayed()).toBe(false);
}

async function exportBrowserConversation(destination: string): Promise<string> {
  await setFolderChoice(destination);
  await $('main [data-conversation-actions]').click();
  const item = $('[data-slot="dropdown-menu-content"] [data-export-conversation]');
  await item.waitForClickable();
  await item.click();
  const exported = $('[data-export-destination]');
  await exported.waitForDisplayed();
  const folder = realpathSync.native(await shownText(await exported));
  expect(dirname(folder)).toBe(destination);
  await browser.keys(Key.Escape);
  return folder;
}

function removeExportDirectory(directory: string): void {
  const temp = realpathSync.native(tmpdir());
  const inTemp = dirname(directory) === temp;
  const isTestFolder = basename(directory).startsWith('mannie-browser-export-');
  if (!inTemp || !isTestFolder) {
    throw new Error('unexpected browser export test folder');
  }
  rmSync(directory, { force: true, maxRetries: 20, recursive: true, retryDelay: 250 });
}

function privateBrowserProfiles(): string[] {
  const directory = join(dataDirectory(), 'browser');
  if (!existsSync(directory)) {
    return [];
  }
  return readdirSync(directory, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && entry.name.startsWith('session-'))
    .map((entry) => entry.name);
}

if (process.platform !== 'win32') {
  it('explains that Browser requires Windows when enabled on an unsupported system', async () => {
    await waitForApp();
    await openSettings('plugins.settings');

    const plugin = '[data-plugin="browser"]';
    await expect($(plugin)).toHaveAttribute('data-plugin-state', 'off');
    await $(`${plugin} [data-plugin-switch]`).click();
    await waitForShownText(`${plugin}[data-plugin-state="failed"] [role="status"]`, /Browser needs Windows/u);
  });
}

const describeBrowserPane = process.platform === 'win32' ? describe : describe.skip;
describeBrowserPane('Browser pane', () => {
  let provider: ProviderServer;
  let site: Site;

  before(async () => {
    provider = await startProviderServer();
    site = await startSite();
    await waitForApp();
    await openSettings('plugins.settings');
    await expect($('[data-plugin="browser"]')).toHaveAttribute('data-plugin-state', 'off');
    await setPluginEnabled('browser', true);
    await leaveSettings();
    await addProfile(provider.baseUrl, { model: MODELS.tools, name: 'Browser tools', useTools: true });
  });

  after(async () => {
    await deleteProviderProfiles();
    await provider.close();
    await closeSite(site.server);
  });

  it('opens Browser settings with keyboard-accessible form help in both themes', async () => {
    await openBrowserConversation('Open the Browser settings fixture.');
    await openBrowserSettings();
    const focused = (selector: string): Promise<boolean> =>
      browser.execute((target) => document.activeElement?.matches(target) ?? false, selector);
    type FormField = readonly [name: string, placeholder: string, label: string, help: string];
    const checkFields = async (dialog: string, fields: readonly [FormField, ...FormField[]]): Promise<void> => {
      const first = $(`${dialog} input[name="${fields[0][0]}"]`);
      await $(dialog).waitForDisplayed();
      await expect(first).toBeFocused();
      await browser.keys([Key.Shift, Key.Tab]);
      for (const [index, [field, placeholder, label, help]] of fields.entries()) {
        if (index > 0) {
          await browser.keys(Key.Tab);
        }
        const helpButton = `${dialog} [data-field-help][aria-label="Help: ${label}"]`;
        expect(await focused(helpButton)).toBe(true);
        const tooltip = $('[data-slot="tooltip-content"]:not([data-state="closed"])');
        await waitForShownText(tooltip, expect.stringContaining(help));
        await browser.keys(Key.Tab);
        const input = $(`${dialog} input[name="${field}"]`);
        await expect(input).toBeFocused();
        await expect(input).toHaveAttribute('placeholder', placeholder);
      }
    };
    const checkThemes = async (): Promise<void> => {
      const originalDark = await isDark();
      try {
        for (const dark of [false, true]) {
          await browser.execute((theme) => document.documentElement.classList.toggle('dark', theme), dark);
          expect(await axeViolations()).toEqual([]);
        }
      } finally {
        await browser.execute((theme) => document.documentElement.classList.toggle('dark', theme), originalDark);
      }
    };

    await $(CERTIFICATES).waitForDisplayed();
    await $(`${CERTIFICATES} [data-slot="empty"] [data-browser-certificate-add]`).click();
    await checkFields(CERTIFICATE_DIALOG, [
      ['hostname', 'example.com', 'Hostname', 'Enter a hostname or IP address'],
      [
        'fingerprint',
        '64 hexadecimal digits',
        'SHA-256 fingerprint',
        "Get the server certificate's SHA-256 fingerprint",
      ],
    ]);
    await checkThemes();
    await browser.keys(Key.Escape);
    await $(CERTIFICATE_DIALOG).waitForDisplayed({ reverse: true });
    await expect($(ADD_CERTIFICATE)).toBeFocused();
    await checkThemes();
    await leaveSettings();

    await openBrowserSettings();
    await $(CREDENTIALS).waitForDisplayed();
    await $(`${CREDENTIALS} [data-slot="empty"] [data-browser-credential-add]`).click();
    await checkFields(CREDENTIAL_DIALOG, [
      ['origin', 'https://example.com', 'Site origin', 'Include the scheme and any port'],
      ['label', 'Account password', 'Label for the agent', 'Use a name the agent can recognize'],
      [
        'value',
        'Enter a password or code',
        'Password or code',
        'The agent can enter this value only at the specified site',
      ],
    ]);
    await checkThemes();
    await clickWithText(`${CREDENTIAL_DIALOG} button`, 'Cancel');
    await $(CREDENTIAL_DIALOG).waitForDisplayed({ reverse: true });
    await expect($(ADD_CREDENTIAL)).toBeFocused();
    await leaveSettings();
    await endBrowserSession();
  });

  it('shows the live page over its pane and reuses a site approval across agent runs', async () => {
    await openBrowserConversation('Open a conversation for the browser fixture.');
    const address = $(OWNER_ADDRESS);
    await ownerNavigate(site.address);
    await waitForSurface(coversPlaceholder, 'the page did not cover its placeholder');
    await browser.waitUntil(async () => (await pageValue('document.title')) === 'Browser fixture', {
      timeoutMsg: 'the page did not load the fixture',
    });
    // The page can load before Mannie finishes the navigation command.
    await address.waitForEnabled();
    expect(await axeViolations()).toEqual([]);

    // The page follows its placeholder when the window changes size.
    const { height, width } = await browser.getWindowSize();
    await browser.setWindowSize(width - 120, height - 80);
    await waitForSurface(coversPlaceholder, 'the page did not follow a window resize');
    await browser.setWindowSize(width, height);
    await waitForSurface(coversPlaceholder, 'the page did not follow the window back');

    const controls = await browser.execute((panel) => {
      const actions = document.querySelector(`${panel} [data-browser-actions]`);
      if (actions === null) {
        return { actions: [], rightGap: null };
      }
      const row = actions.parentElement;
      if (row === null) {
        return { actions: [], rightGap: null };
      }
      const buttons = Array.from(actions.querySelectorAll('button'), (button) =>
        (button.getAttribute('aria-label') ?? button.textContent).trim(),
      );
      return {
        actions: buttons,
        rightGap: row.getBoundingClientRect().right - actions.getBoundingClientRect().right,
      };
    }, PANEL);
    expect(controls.actions).toEqual(['Resume agent', 'End session']);
    expect(controls.rightGap).not.toBeNull();
    expect(controls.rightGap).toBeCloseTo(0, 0);

    const watched = await shownTab();
    await $(
      `[data-side-panel-tab="browser.session:${watched.conversationId}.${watched.tabId}"] [data-side-panel-close]`,
    ).click();
    await $(PANEL).waitForDisplayed({ reverse: true });
    await $('[data-browser-control]').click();
    await clickWithText('[data-browser-open-tab]', 'Browser fixture');
    await $(PANEL).waitForDisplayed();
    expect((await shownTab()).tabId).toBe(watched.tabId);
    await waitForSurface(coversPlaceholder, 'the Browser menu did not reopen the existing tab');

    // An open dialog hides the page, and the pane shows a still in its place.
    await clickWithText(`${PANEL} button:not(:disabled)`, 'End session');
    await waitForSurface((surface) => !surface.visible, 'the page stayed over the session confirmation');
    await $(STILL).waitForDisplayed();
    await clickWithText('[role="alertdialog"] button', 'Cancel');
    await waitForSurface(coversPlaceholder, 'the page did not return after the dialog closed');

    // WebDriver can send F6 from the app webview. Leaving the native page
    // with its own keys is a by-hand check.
    await address.click();
    await browser.keys(Key.F6);
    await waitForSurface((surface) => surface.focused, 'focus did not enter the page');
    await clickWithText(`${PANEL} button:not(:disabled)`, 'Resume agent');
    await waitForShownText($(PANEL), expect.stringContaining('Agent control'));
    await waitForSurface(
      (surface) => !surface.enabled && !surface.focused,
      'the page kept the focus or owner input under agent control',
    );
    await clickWithText(`${PANEL} button:not(:disabled)`, 'Take over');
    await waitForShownText($(PANEL), expect.stringContaining('You have control'));
    await waitForSurface((surface) => surface.enabled, 'the page refused the owner in takeover');

    // A reload of the app page ends the lease, and the pane takes it again.
    await browser.refresh();
    await waitForApp();
    await $(PANEL).waitForDisplayed({ timeout: 20_000 });
    await waitForSurface(coversPlaceholder, 'the page did not return after the app reloaded');

    await clickWithText(`${PANEL} button:not(:disabled)`, 'End session');
    await clickWithText('[role="alertdialog"] button', 'End session');
    await waitForShownText($(PANEL), expect.stringContaining('No browser session'));
    await $(PANEL).$('button=Start browser').waitForEnabled();

    await sendLines([
      callMessage('browser_navigate', { action: { kind: 'go', url: `${site.address}/agent-approved` } }),
      'next',
      callMessage('browser_observe', { screenshot: false }),
    ]);
    await waitForToolStatus('awaiting_approval');
    expect(site.requests).not.toContain('/agent-approved');
    await clickInLatestTurn('Allow for this conversation');
    await waitForToolStatus('completed');
    const arrived = await waitForCompleteReply();
    expect(arrived).toContain('Browser fixture');
    expect(site.requests).toContain('/agent-approved');
    const ready = await waitForSurface(coversPlaceholder, 'the agent page did not appear');
    expect(ready.cursor).toBeNull();

    const firstClick = await clickBrowserTarget(observation(arrived), 'First target', 'First pressed', true);
    await waitForShownText($(ACTIVITY), 'The agent clicked on the page.');
    await clickBrowserTarget(observation(firstClick), 'Second target', 'Second pressed');

    await clickWithText(`${PANEL} button:not(:disabled)`, 'Take over');
    await waitForShownText($(PANEL), expect.stringContaining('You have control'));
    await expect($(ACTIVITY)).toHaveText('');
    const takenOver = await waitForSurface((surface) => surface.enabled, 'the page refused the owner in takeover');
    expect(takenOver.cursor).toBeNull();
    const otherOrigin = site.address.replace('127.0.0.1', 'localhost');
    const deniedNavigation = callMessage('browser_navigate', {
      action: { kind: 'go', url: `${otherOrigin}/agent-denied` },
    });
    await send(deniedNavigation);
    await waitForToolStatus('awaiting_approval');
    await waitForShownText($(PANEL), expect.stringContaining('You have control'));
    await clickInLatestTurn('Give agent control');
    expect(observation(await waitForCompleteReply()).requestRan).toBe(false);
    await waitForShownText($(PANEL), expect.stringContaining('Agent control'));
    expect(site.requests).not.toContain('/agent-denied');

    await send(deniedNavigation);
    await waitForToolStatus('awaiting_approval');
    await clickInLatestTurn('Deny');
    await waitForCompleteReply();
    expect(site.requests).not.toContain('/agent-denied');

    const beforeScreenshot = provider.requests().length;
    const watchedTab = await $(PANEL).getAttribute('data-browser-tab');
    if (watchedTab === null) {
      throw new Error('the visible Browser panel does not address a tab');
    }
    await sendLines([
      callMessage('browser_navigate', { action: { kind: 'open_tab', url: `${site.address}/todos` } }),
      'next',
      callMessage('browser_observe', { screenshot: true }),
    ]);
    const todos = observation(await waitForCompleteReply());
    expect(todos.screenshot).toMatchObject({ status: 'returned' });
    const { height: shotHeight, width: shotWidth } = todos.screenshot as { height: number; width: number };
    expect(shotWidth).toBeGreaterThan(0);
    expect(shotHeight).toBeGreaterThan(0);
    const screenshotRequests = provider.requests().slice(beforeScreenshot);
    expect(screenshotRequests.some((request) => JSON.stringify(request.body).includes('data:image/jpeg;base64,'))).toBe(
      true,
    );
    let labels: string[] = [];
    try {
      await browser.waitUntil(async () => {
        labels = await $$(SHELL_TABS).map((tab) => tab.getText());
        return labels.includes('Browser fixture') && labels.includes('Batch todos');
      });
    } catch (error) {
      throw new Error(`the shared strip did not show both Browser tabs. It showed ${JSON.stringify(labels)}`, {
        cause: error,
      });
    }
    await expect($(PANEL)).toHaveAttribute('data-browser-tab', watchedTab);
    await clickWithText(SHELL_TABS, 'Batch todos');

    const checkboxes = todoNames.map((name) => namedEntry(todos.outline, 'name', `Mark ${name} complete`));
    expect(checkboxes.map((checkbox) => checkbox.checked)).toEqual([true, false, false]);
    const references = checkboxes
      .filter((checkbox) => checkbox.checked === false)
      .map((checkbox) => textField(checkbox, 'reference'));
    await sendLines([
      callMessage('browser_interact', {
        action: { kind: 'click_many', references },
        revision: textField(todos, 'revision'),
      }),
      'next',
      callMessage('browser_observe', { screenshot: false }),
    ]);
    expect(await waitForCompleteReply()).toContain('3 of 3 complete');

    await runCommand('Show the permissions of this conversation');
    const permissions = '[data-slot="dialog-content"]';
    await waitForShownText($(permissions), expect.stringContaining(site.address));
    await clickWithText(`${permissions} button`, 'Revoke');
    await expect($(permissions)).not.toHaveText(expect.stringContaining(site.address));
    await clickWithText(`${permissions} button`, 'Close');
    await $(permissions).waitForDisplayed({ reverse: true });

    await send(callMessage('browser_navigate', { action: { kind: 'go', url: `${site.address}/after-revoke` } }));
    await waitForToolStatus('awaiting_approval');
    expect(site.requests).not.toContain('/after-revoke');
    await clickInLatestTurn('Deny');
    await waitForCompleteReply();
  });

  it('requires the owner to trust the exact certificate and keeps agent approval separate', async () => {
    const certificate = selfSignedCertificate();
    const replacement = selfSignedCertificate();
    const first = await startHttpsSite(certificate);
    try {
      const second = await startHttpsSite(certificate);
      try {
        const hostname = new URL(first.address).hostname;
        const sameCertificateOtherHost = first.address.replace('127.0.0.1', 'localhost');
        first.redirectTo = `${second.address}/redirect-destination`;
        first.untrustedAsset = `${sameCertificateOtherHost}/background.js`;
        expect(fixtureCertificateInWindowsTrust(certificate)).toBe(false);

        await openBrowserConversation('Open a conversation for the HTTPS fixture.');
        await ownerNavigate('file:///wrong-scheme');
        const dismiss = $(`${PANEL} [data-browser-dismiss-error]`);
        await dismiss.waitForClickable();
        await dismiss.click();
        await dismiss.waitForExist({ reverse: true });
        await ownerNavigate(`${first.address}/without-exception`);
        await waitForCertificateError();
        expect(first.requests).toEqual([]);

        await trustCertificate(hostname, replacement.fingerprint);
        await ownerNavigate(`${first.address}/wrong-fingerprint`);
        await waitForCertificateError();
        expect(first.requests).toEqual([]);
        await $(`${CERTIFICATE_ERROR} [data-browser-dismiss-certificate-error]`).click();
        await $(CERTIFICATE_ERROR).waitForDisplayed({ reverse: true });
        await forgetCertificate(replacement.fingerprint);

        await trustCertificate(hostname, certificate.fingerprint);
        expect(first.requests).toEqual([]);
        await ownerNavigate(`${first.address.replace('https://', '')}/with-background`);
        await waitForHttpsFixture('/with-background');
        expect(first.requests).toEqual(expect.arrayContaining(['/with-background', '/fixture.js']));
        await browser.waitUntil(
          async () => (await pageValue('document.documentElement.dataset.background')) === 'blocked',
          {
            timeoutMsg: 'the asset at the untrusted hostname did not fail',
          },
        );
        expect(first.requests).not.toContain('/background.js');
        expect(await $(CERTIFICATE_DIALOG).isDisplayed()).toBe(false);
        expect(await $(CERTIFICATE_ERROR).isDisplayed()).toBe(false);

        await ownerNavigate(`${sameCertificateOtherHost}/other-host`);
        await waitForCertificateError();
        expect(first.requests).not.toContain('/other-host');

        await clickWithText(`${PANEL} button:not(:disabled)`, 'Resume agent');
        await sendLines([
          callMessage('browser_navigate', { action: { kind: 'go', url: `${first.address}/agent-site` } }),
          'next',
          callMessage('browser_observe', { screenshot: false }),
        ]);
        await waitForToolStatus('awaiting_approval');
        expect(first.requests).not.toContain('/agent-site');
        await clickInLatestTurn('Allow for this conversation');
        expect(await waitForCompleteReply()).toContain('Self-signed fixture');
        expect(first.requests).toContain('/agent-site');

        await send(callMessage('browser_navigate', { action: { kind: 'go', url: `${first.address}/redirect` } }));
        await waitForToolStatus('awaiting_approval');
        expect(first.requests).toContain('/redirect');
        expect(second.requests).toEqual([]);
        await clickInLatestTurn('Deny');
        await waitForCompleteReply();
        expect(second.requests).toEqual([]);

        await sendLines([
          callMessage('browser_navigate', { action: { kind: 'go', url: `${second.address}/another-port` } }),
          'next',
          callMessage('browser_observe', { screenshot: false }),
        ]);
        expect(await allowBrowserSite()).toContain('Self-signed fixture');
        expect(second.requests).toContain('/another-port');

        await clickWithText(`${PANEL} button:not(:disabled)`, 'Take over');
        await waitForShownText($(PANEL), expect.stringContaining('You have control'));
        first.rotate(replacement);
        await ownerNavigate(`${first.address}/rotated`);
        await waitForCertificateError();
        expect(first.requests).not.toContain('/rotated');

        await forgetCertificate(certificate.fingerprint);
        await waitForShownText($(PANEL), expect.stringContaining('removed a certificate it trusted'));
        await startBrowserSession();
        await ownerNavigate(`${second.address}/new-session`);
        await waitForCertificateError();
        expect(second.requests).not.toContain('/new-session');
        await trustCertificate(hostname, replacement.fingerprint);
        await ownerNavigate(`${first.address}/rotated`);
        await waitForHttpsFixture('/rotated');
        expect(first.requests).toContain('/rotated');
        expect(fixtureCertificateInWindowsTrust(certificate)).toBe(false);
        expect(fixtureCertificateInWindowsTrust(replacement)).toBe(false);

        // Saved trust applies in a separate conversation after its own site approval.
        await openBrowserConversation('Open a separate conversation for the same HTTPS fixture.');
        await clickWithText(`${PANEL} button:not(:disabled)`, 'Resume agent');
        await sendLines([
          callMessage('browser_navigate', { action: { kind: 'go', url: `${first.address}/agent-saved-pin` } }),
          'next',
          callMessage('browser_observe', { screenshot: false }),
        ]);
        expect(await allowBrowserSite()).toContain('Self-signed fixture');
        expect(first.requests).toContain('/agent-saved-pin');
        await endBrowserSession();
      } finally {
        try {
          await forgetCertificate(certificate.fingerprint);
          await forgetCertificate(replacement.fingerprint);
        } finally {
          await second.close();
        }
      }
    } finally {
      await first.close();
    }
  });

  it('ends the Browser session and closes a live HTTPS connection before an allowed certificate expires', async () => {
    await openBrowserConversation('Open a conversation for the short-lived HTTPS fixture.');
    const certificate = selfSignedCertificate(35);
    const shortLived = await startHttpsSite(certificate);
    try {
      await trustCertificate(new URL(shortLived.address).hostname, certificate.fingerprint);
      await ownerNavigate(`${shortLived.address}/keep-connection`);
      await waitForHttpsFixture('/keep-connection');
      await browser.waitUntil(() => shortLived.requests.includes('/hold'), {
        timeoutMsg: 'the page did not open a long-lived HTTPS response',
      });
      expect(shortLived.holdClosedAt()).toBeUndefined();

      await browser.waitUntil(() => shortLived.holdClosedAt() !== undefined, {
        timeout: Math.max(1_000, certificate.notAfter - Date.now() + 5_000),
        timeoutMsg: 'the connection stayed open after certificate expiry',
      });
      expect(shortLived.holdClosedAt()).toBeLessThan(certificate.notAfter);
      await waitForShownText($(PANEL), expect.stringContaining('A trusted certificate was about to expire'));

      await startBrowserSession();
      await ownerNavigate(`${shortLived.address}/after-cutoff`);
      await waitForCertificateError();
      expect(shortLived.requests).not.toContain('/after-cutoff');
    } finally {
      try {
        await forgetCertificate(certificate.fingerprint);
      } finally {
        await shortLived.close();
      }
    }
  });

  it('reuses saved certificate trust after a restart and closes live connections when forgotten', async () => {
    const certificate = selfSignedCertificate();
    const replacement = selfSignedCertificate();
    const saved = await startHttpsSite(certificate);
    try {
      const hostname = new URL(saved.address).hostname;
      await openBrowserConversation('Open a conversation to save HTTPS trust.');
      await ownerNavigate(`${saved.address}/before-save`);
      await waitForCertificateError();
      expect(saved.requests).toEqual([]);
      await trustCertificate(hostname, certificate.fingerprint);
      await ownerNavigate(`${saved.address}/after-save`);
      await waitForHttpsFixture('/after-save');
      await endBrowserSession();
      await startBrowserSession();
      await ownerNavigate(`${saved.address}/after-session-end`);
      await waitForHttpsFixture('/after-session-end');

      await restartMannie();
      await $('[data-new-conversation]').waitForClickable();
      await openBrowserConversation('Open a new conversation after restarting Mannie.');
      await clickWithText(`${PANEL} button:not(:disabled)`, 'Resume agent');
      await sendLines([
        callMessage('browser_navigate', { action: { kind: 'go', url: `${saved.address}/after-restart` } }),
        'next',
        callMessage('browser_observe', { screenshot: false }),
      ]);
      await waitForToolStatus('awaiting_approval');
      expect(saved.requests).not.toContain('/after-restart');
      await clickInLatestTurn('Allow for this conversation');
      expect(await waitForCompleteReply()).toContain('Self-signed fixture');
      expect(saved.requests).toContain('/after-restart');

      await clickWithText(`${PANEL} button:not(:disabled)`, 'Take over');
      await waitForShownText($(PANEL), expect.stringContaining('You have control'));
      saved.rotate(replacement);
      await ownerNavigate(`${saved.address}/changed-fingerprint`);
      await waitForCertificateError();
      expect(saved.requests).not.toContain('/changed-fingerprint');

      saved.rotate(certificate);
      await ownerNavigate(`${saved.address}/keep-connection`);
      await waitForHttpsFixture('/keep-connection');
      await browser.waitUntil(() => saved.requests.includes('/hold'), {
        timeoutMsg: 'the page did not open a long-lived HTTPS response',
      });
      expect(saved.holdClosedAt()).toBeUndefined();

      await openBrowserSettings();
      const entry = $(`${CERTIFICATES} [data-slot="item-group"] [role="listitem"][data-browser-saved-certificate]`);
      await expect(entry).toHaveText(expect.stringContaining(hostname));
      await expect(entry).toHaveText(
        expect.stringContaining(certificate.fingerprint.replaceAll(':', '').toLowerCase()),
      );
      await $(ADD_CERTIFICATE).click();
      await $(CERTIFICATE_DIALOG).waitForDisplayed();
      await clickWithText(`${CERTIFICATE_DIALOG} button`, 'Cancel');
      await $(CERTIFICATE_DIALOG).waitForDisplayed({ reverse: true });
      await expect($(ADD_CERTIFICATE)).toBeFocused();
      await entry.$('[data-browser-certificate-forget]').click();
      await clickWithText('[role="alertdialog"] button', 'Cancel');
      await $('[role="alertdialog"]').waitForDisplayed({ reverse: true });
      expect(saved.holdClosedAt()).toBeUndefined();
      await expect(entry).toBeDisplayed();

      await entry.$('[data-browser-certificate-forget]').click();
      await $('[role="alertdialog"] [data-browser-certificate-forget-confirm]').click();
      await entry.waitForExist({ reverse: true });
      await waitForShownText($(CERTIFICATES), expect.stringContaining('No saved certificates.'));
      await leaveSettings();
      await waitForShownText(
        $(PANEL),
        expect.stringContaining('Browser ended this session because you removed a certificate'),
      );
      await browser.waitUntil(() => saved.holdClosedAt() !== undefined, {
        timeoutMsg: 'the connection stayed open after the saved trust was removed',
      });

      await startBrowserSession();
      await ownerNavigate(`${saved.address}/after-forgetting`);
      await waitForCertificateError();
      expect(saved.requests).not.toContain('/after-forgetting');
      expect(fixtureCertificateInWindowsTrust(certificate)).toBe(false);
      await endBrowserSession();
    } finally {
      try {
        await forgetCertificate(certificate.fingerprint);
      } finally {
        await saved.close();
      }
    }
  });

  it('uses a saved site credential with conversation approval without exposing its value', async () => {
    const earlierProfiles = privateBrowserProfiles();
    await openBrowserConversation('Open a conversation for the protected sign-in fixture.');
    const newProfiles = privateBrowserProfiles().filter((name) => !earlierProfiles.includes(name));
    expect(newProfiles).toHaveLength(1);

    const secret = `browser-fixture-${randomUUID()}`;
    // The panel moves to the new session's tab first.
    await $(`${PANEL}[data-browser-tab]`).waitForExist();
    await openBrowserSettings();
    await saveCredential(site.address, 'Fixture password', secret);
    await leaveSettings();
    await clickWithText(`${PANEL} button:not(:disabled)`, 'Resume agent');

    await sendLines([
      callMessage('browser_navigate', { action: { kind: 'go', url: `${site.address}/sign-in` } }),
      'next',
      callMessage('browser_observe', { screenshot: false }),
    ]);
    const initial = observation(await allowBrowserSite());

    const credential = reference(initial.credentials, 'label', 'Fixture password');
    await sendLines([
      callMessage('browser_interact', {
        action: {
          kind: 'fill',
          reference: reference(initial.outline, 'name', 'Password'),
          value: { kind: 'credential', reference: credential },
        },
        revision: textField(initial, 'revision'),
      }),
      'next',
      callMessage('browser_observe', { screenshot: true }),
    ]);
    await waitForToolStatus('awaiting_approval');
    await waitForShownText(CREDENTIAL_APPROVAL, 'Fixture password');
    expect(site.signIns).toEqual([]);
    await clickInLatestTurn('Allow for this conversation');
    const protectedReply = await waitForCompleteReply();
    expect(protectedReply).toContain('[redacted]');
    expect(protectedReply).not.toContain(secret);
    await waitForShownText($(ACTIVITY), 'The agent filled in a field.');
    const filled = observation(protectedReply);
    expect(filled.screenshot).toMatchObject({ status: 'withheld' });

    await sendLines([
      callMessage('browser_interact', {
        action: { kind: 'click', target: { kind: 'element', reference: reference(filled.outline, 'name', 'Sign in') } },
        revision: textField(filled, 'revision'),
      }),
      'next',
      callMessage('browser_observe', { screenshot: false }),
    ]);
    const signedIn = await waitForCompleteReply();
    expect(signedIn).toContain('Signed in');
    expect(signedIn).toContain('Received [redacted]');
    expect(signedIn).not.toContain(secret);
    expect(site.signIns).toEqual([new URLSearchParams({ password: secret }).toString()]);

    await browser.refresh();
    await waitForApp();
    expect(await waitForCompleteReply()).toContain('Received [redacted]');
    const transcript = await browser.execute(
      (selector) => [...document.querySelectorAll(selector)].map((turn) => turn.textContent).join('\n'),
      MAIN_TURNS,
    );
    expect(transcript).not.toContain(secret);
    expect(JSON.stringify(provider.requests().map((request) => request.body))).not.toContain(secret);

    const destination = realpathSync.native(mkdtempSync(join(tmpdir(), 'mannie-browser-export-')));
    try {
      const exported = await exportBrowserConversation(destination);
      const text = readFileSync(join(exported, 'conversation.md'), 'utf8');
      expect(text).toContain('browser_interact');
      expect(text).toContain('Received [redacted]');
      const files = readdirSync(exported, { recursive: true, withFileTypes: true }).filter((entry) => entry.isFile());
      expect(files.some((entry) => /\.(?:jpeg|jpg|png)$/iu.test(entry.name))).toBe(false);
      for (const file of files) {
        expect(readFileSync(join(file.parentPath, file.name)).includes(secret)).toBe(false);
      }
    } finally {
      removeExportDirectory(destination);
    }

    expect(privateBrowserProfiles()).toEqual(expect.arrayContaining(newProfiles));
    await restartMannie();

    await openBrowserConversation('Open a second conversation for the protected sign-in fixture.');
    await clickWithText(`${PANEL} button:not(:disabled)`, 'Resume agent');
    await sendLines([
      callMessage('browser_navigate', { action: { kind: 'go', url: `${site.address}/sign-in` } }),
      'next',
      callMessage('browser_observe', { screenshot: false }),
    ]);
    const second = observation(await allowBrowserSite());
    expect(reference(second.credentials, 'label', 'Fixture password')).toBe(credential);
    const fill = callMessage('browser_interact', {
      action: {
        kind: 'fill',
        reference: reference(second.outline, 'name', 'Password'),
        value: { kind: 'credential', reference: credential },
      },
      revision: textField(second, 'revision'),
    });
    await send(fill);
    await waitForToolStatus('awaiting_approval');
    await waitForShownText(CREDENTIAL_APPROVAL, 'Fixture password');
    await clickInLatestTurn('Deny');
    await waitForCompleteReply();
    expect(site.signIns).toHaveLength(1);

    await openBrowserSettings();
    const credentialRow = $(`${CREDENTIALS} [data-slot="item-group"] [role="listitem"]`);
    await waitForShownText(credentialRow, expect.stringContaining('Fixture password'));
    await $(ADD_CREDENTIAL).click();
    await $(CREDENTIAL_DIALOG).waitForDisplayed();
    await clickWithText(`${CREDENTIAL_DIALOG} button`, 'Cancel');
    await $(CREDENTIAL_DIALOG).waitForDisplayed({ reverse: true });
    await expect($(ADD_CREDENTIAL)).toBeFocused();
    await credentialRow.$('[data-browser-credential-remove]').click();
    await credentialRow.waitForExist({ reverse: true });
    await leaveSettings();
    await send(callMessage('browser_observe', { screenshot: false }));
    const removed = observation(await waitForCompleteReply());
    expect(removed.credentials).toEqual([]);
    await send(
      callMessage('browser_interact', {
        action: {
          kind: 'fill',
          reference: reference(removed.outline, 'name', 'Password'),
          value: { kind: 'credential', reference: credential },
        },
        revision: textField(removed, 'revision'),
      }),
    );
    expect(await waitForCompleteReply()).toContain('this credential is unavailable at the field');
    expect(site.signIns).toHaveLength(1);

    await openSettings('plugins.settings');
    await $('[data-plugin="browser"] [data-plugin-switch]').click();
    const confirmation = $('[data-plugin-off-confirm]');
    await confirmation.waitForClickable();
    await confirmation.click();
    await $('[data-plugin="browser"][data-plugin-state="off"]').waitForExist({ timeout: 30_000 });
    await browser.waitUntil(() => privateBrowserProfiles().length === 0, {
      timeout: 30_000,
      timeoutMsg: 'disabling Browser did not remove its private profiles',
    });
  });
});
