import { randomUUID } from 'node:crypto';
import { existsSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync } from 'node:fs';
import { createServer, type Server } from 'node:http';
import { tmpdir } from 'node:os';
import { basename, dirname, join } from 'node:path';

import { dataDirectory } from '#e2e/app.ts';
import {
  addProfile,
  callMessage,
  clickInLatestTurn,
  clickWithText,
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
  deleteProviderProfiles,
  setFolderChoice,
} from '#e2e/harness.ts';
import {
  axeViolations,
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

const PANEL = '[data-side-panels] [role="tabpanel"][data-state="active"] [data-browser-panel]';
const VIEWPORT = `${PANEL} [data-browser-viewport]`;
const STILL = `${VIEWPORT} img`;
const ACTIVITY = `${PANEL} [data-browser-activity]`;
const SHELL_TABS = '[data-side-panels] [role="tablist"] [role="tab"]';

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

  it('shows the live page over its pane and reuses a site approval across agent runs', async () => {
    await startNewConversation();
    await sendMessage('Open a conversation for the browser fixture.');
    await waitForCompleteReply();
    await $('[data-browser-control]').click();
    await clickWithText(`${PANEL} button:not(:disabled)`, 'Start browser');
    await waitForShownText($(PANEL), expect.stringContaining('You have control'));

    const address = $(`${PANEL} [aria-label="Address"]:not([readonly])`);
    await address.setValue(site.address);
    await clickWithText(`${PANEL} button:not(:disabled)`, 'Go');
    await waitForSurface(coversPlaceholder, 'the page did not cover its placeholder');
    await browser.waitUntil(async () => (await pageValue('document.title')) === 'Browser fixture', {
      timeoutMsg: 'the page did not load the fixture',
    });
    // The page can load before Mannie finishes the navigation command.
    await $(`${PANEL} button[type="submit"]`).waitForEnabled();
    expect(await axeViolations()).toEqual([]);

    // The page follows its placeholder when the window changes size.
    const { height, width } = await browser.getWindowSize();
    await browser.setWindowSize(width - 120, height - 80);
    await waitForSurface(coversPlaceholder, 'the page did not follow a window resize');
    await browser.setWindowSize(width, height);
    await waitForSurface(coversPlaceholder, 'the page did not follow the window back');

    // An open dialog hides the page, and the pane shows a still in its place.
    await $(`${PANEL} [data-browser-credentials]`).click();
    await waitForSurface((surface) => !surface.visible, 'the page stayed over the credentials dialog');
    await $(STILL).waitForDisplayed();
    await $('[role="dialog"] input[name="origin"]').setValue(site.address);
    await $('[role="dialog"] input[name="label"]').setValue('Fixture code');
    await $('[role="dialog"] input[name="value"]').setValue('123456');
    await clickWithText('[role="dialog"] button', 'Add to session');
    await waitForShownText($('[role="dialog"]'), expect.stringContaining('Fixture code'));
    expect(await $('[role="dialog"]').getText()).not.toContain('123456');
    await clickWithText('[role="dialog"] button:not(:disabled)', 'Remove Fixture code');
    await clickWithText('[role="dialog"] button', 'Close');
    await waitForSurface(coversPlaceholder, 'the page did not return after the dialog closed');

    // Focus enters the page. Resume takes it out before the page stops
    // taking owner input. F6, Tab and typing in the page are by-hand checks:
    // WebDriver's keys reach only the app webview.
    await clickWithText(`${PANEL} button:not(:disabled)`, 'Focus page');
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

  it('uses a session password without retaining it in the transcript or export', async () => {
    await startNewConversation();
    await sendMessage('Open a conversation for the protected sign-in fixture.');
    await waitForCompleteReply();
    const earlierProfiles = privateBrowserProfiles();
    await $('[data-browser-control]').click();
    await clickWithText(`${PANEL} button:not(:disabled)`, 'Start browser');
    await waitForShownText($(PANEL), expect.stringContaining('You have control'));
    const newProfiles = privateBrowserProfiles().filter((name) => !earlierProfiles.includes(name));
    expect(newProfiles).toHaveLength(1);

    const secret = `browser-fixture-${randomUUID()}`;
    // The panel moves to the new session's tab first.
    await $(`${PANEL}[data-browser-tab]`).waitForExist();
    await $(`${PANEL} [data-browser-credentials]`).click();
    await $('[role="dialog"] input[name="origin"]').setValue(site.address);
    await $('[role="dialog"] input[name="label"]').setValue('Fixture password');
    await $('[role="dialog"] input[name="value"]').setValue(secret);
    await clickWithText('[role="dialog"] button', 'Add to session');
    await waitForShownText($('[role="dialog"]'), expect.stringContaining('Fixture password'));
    await clickWithText('[role="dialog"] button', 'Close');
    await clickWithText(`${PANEL} button:not(:disabled)`, 'Resume agent');

    await sendLines([
      callMessage('browser_navigate', { action: { kind: 'go', url: `${site.address}/sign-in` } }),
      'next',
      callMessage('browser_observe', { screenshot: false }),
    ]);
    const initial = observation(await allowBrowserSite());

    await sendLines([
      callMessage('browser_interact', {
        action: {
          kind: 'fill',
          reference: reference(initial.outline, 'name', 'Password'),
          value: { kind: 'credential', reference: reference(initial.credentials, 'label', 'Fixture password') },
        },
        revision: textField(initial, 'revision'),
      }),
      'next',
      callMessage('browser_observe', { screenshot: true }),
    ]);
    const protectedReply = await waitForCompleteReply();
    expect(protectedReply).toContain('[redacted]');
    expect(protectedReply).not.toContain(secret);
    await waitForShownText($(ACTIVITY), 'The agent filled in a field.');
    const filled = observation(protectedReply);

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
