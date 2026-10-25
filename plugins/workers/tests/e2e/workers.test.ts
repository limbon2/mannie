// Workers in the app (plugins/workers/docs/worker-activity.md, acceptance),
// with a real worker on this computer: the pairing dialog shows its fields'
// help on keyboard focus; pairing refuses a wrong code and pairs with the
// right one under the name that the owner gave; a delegated task shows its
// card and its request with that name, the owner allows the request in the
// card, and its conversation opens live in a side panel; Stop ends a task on
// the worker; a worker that stops shows as offline and as connected once it
// is back; an empty name in "Rename…" goes back to the worker's configured
// name; and removing a worker asks first. Every step works from the keyboard
// and passes axe in both themes. The fixture provider's tools scenario calls
// the tools that each message names, and a task's instruction scripts the
// worker agent the same way. Workers is a plugin that a new installation
// has off (docs/features/plugins.md), with the sensitive services and the
// secret entry that it may use on the Plugins page, so the spec turns it on
// first. Its settings show only catalog text in the pseudo-locale.
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

import {
  addProfile,
  callMessage,
  clickInLatestTurn,
  clickWithText,
  LATEST_MAIN_TURN,
  send,
  startNewConversation,
  waitForCompleteReply,
  waitForToolStatus,
} from '#e2e/chat.ts';
import { deleteProviderProfiles } from '#e2e/harness.ts';
import {
  axeViolations,
  focusedElement,
  leaveSettings,
  openPluginUses,
  openSettings,
  runCommand,
  setPluginEnabled,
  switchToPseudoLocale,
  uncataloguedText,
  waitForAnimations,
  waitForApp,
  waitForShownText,
} from '#e2e/page.ts';
import { MODELS, type ProviderServer, startProviderServer, TOOLS_DONE } from '#e2e/provider-server.ts';
import { Key } from 'webdriverio';

import { TestWorker } from './support/worker.ts';

/** The Workers plugin's row on the Plugins page. */
const PLUGIN = '[data-plugin="workers"]';

/** The worker's name, which it gives itself in its configuration. */
const WORKER_NAME = 'e2e-worker';

/** The name that the owner gives the worker when pairing it, which Mannie and the conversation agent use. */
const OWNER_NAME = 'e2e-named';

/** The sidebar's group of workers. */
const PANEL = '[data-workers-panel]';

/** The side panel area. */
const PANELS = '[data-side-panels]';

/** The card of the task with this number in the latest turn of the main view. */
function card(number: number): string {
  return `${LATEST_MAIN_TURN} [data-task-number="${String(number)}"]`;
}

/** The line of a message that makes the conversation agent delegate a task to the worker, by the owner's name for it. */
function delegate(title: string, instruction: string): string {
  return callMessage('delegate_task', {
    background: false,
    instruction,
    read_only: false,
    title,
    worker: OWNER_NAME,
  });
}

/** Waits until the card of a task shows the task in `state`. */
async function waitForTaskState(number: number, state: string, timeout = 30_000): Promise<void> {
  await $(`${card(number)}[data-task-state="${state}"]`).waitForExist({
    timeout,
    timeoutMsg: `task ${String(number)} did not become ${state}`,
  });
}

/** Waits until the sidebar shows the worker with `status`. */
async function waitForWorkerStatus(status: string, timeout = 30_000): Promise<void> {
  await $(`${PANEL} [data-worker-status="${status}"]`).waitForExist({
    timeout,
    timeoutMsg: `the worker did not show as ${status}`,
  });
}

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

describe('Workers on the Plugins page', () => {
  before(async () => {
    await waitForApp();
  });

  it('lists Workers off in a new installation, with the sensitive services, the secret entry, the worker and the sockets that it may use', async () => {
    await openSettings('plugins.settings');
    await expect($(PLUGIN)).toHaveAttribute('data-plugin-state', 'off');

    await openPluginUses('workers');

    for (const sensitive of ['credentials', 'secrets']) {
      await expect($(`${PLUGIN} [data-plugin-service="${sensitive}"] [data-plugin-service-sensitive]`)).toBeDisplayed();
    }
    await expect($(`${PLUGIN} [data-plugin-service="store"] [data-plugin-service-sensitive]`)).not.toBeExisting();
    await expect($(`${PLUGIN} [data-plugin-secret="link/master-key"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-command="pair_worker"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-channel="subscribe_workers"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-program="mannie-worker"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-crate="tonic"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-native="sockets"]`)).toBeDisplayed();
    await axeInBothThemes();
    await leaveSettings();
  });
});

describe('workers', () => {
  let provider: ProviderServer;
  let worker: TestWorker;

  before(async () => {
    provider = await startProviderServer();
    worker = await TestWorker.start(WORKER_NAME);
    await waitForApp();
    await setPluginEnabled('workers', true);
    await leaveSettings();
    await addProfile(provider.baseUrl, { model: MODELS.tools, name: 'Tools', useTools: true });
  });

  after(async () => {
    await worker.stop();
    worker.remove();
    // The profiles' credentials live in the system's credential store.
    await deleteProviderProfiles();
    await provider.close();
  });

  it('shows no workers group before a worker is paired, and explains pairing in the settings', async () => {
    await expect($(PANEL)).not.toBeExisting();

    await openSettings('workers.settings');

    await waitForShownText($('main'), expect.stringContaining('No workers'));
    await expect($('main [data-pair-worker-settings]')).toBeDisplayed();
    await axeInBothThemes();
    await leaveSettings();
  });

  it('opens the pairing dialog in the Address field, and shows a field’s help on keyboard focus', async () => {
    await runCommand('Pair a worker…');
    await $('[data-pair-worker-dialog]').waitForDisplayed();
    const address = $('#pair-worker-address');
    await expect(address).toBeFocused();

    // The address's help button comes just before the field.
    await browser.keys([Key.Shift, Key.Tab]);

    expect(await focusedElement()).toEqual({ label: 'Help: Address', tag: 'button' });
    await waitForShownText('[data-slot="tooltip-content"]', /with the port when it is not 7445/u);
    await expect(address).toHaveAttribute('aria-describedby', 'pair-worker-address-help');
    expect(await $('#pair-worker-address-help').getProperty('textContent')).toMatch(
      /with the port when it is not 7445/u,
    );
    await browser.keys(Key.Tab);
    await expect(address).toBeFocused();
  });

  it('refuses a wrong code with what to do, and pairs with the code from the host under the owner’s name', async () => {
    await fill('#pair-worker-address', worker.address);
    await fill('#pair-worker-code', 'AAAA-AAAA-AAAA-AAAA-AAAA');
    await browser.keys(Key.Enter);

    await waitForShownText('[data-pair-worker-dialog] [role="alert"]', /refused the code/u);
    await waitForAnimations();
    await axeInBothThemes();

    await fill('#pair-worker-code', worker.code());
    await fill('#pair-worker-name', OWNER_NAME);
    await browser.keys(Key.Enter);

    await $('[data-pair-worker-dialog]').waitForExist({ reverse: true, timeout: 20_000 });
    await waitForWorkerStatus('connected');
    await waitForShownText($(`${PANEL} [data-worker-row]`), expect.stringContaining(OWNER_NAME));
    await axeInBothThemes();
  });

  it('shows the worker in the settings with what it said about its host', async () => {
    await openSettings('workers.settings');
    const details = $('main [data-worker-details]');
    await waitForShownText(details, expect.stringContaining(OWNER_NAME));
    await waitForShownText(details, expect.stringContaining(worker.address));

    await $('main [data-worker-more]').click();

    await waitForShownText(details, expect.stringContaining('Runs the end-to-end tests'));
    await waitForShownText(details, expect.stringContaining('Asks before every change and command'));
    await axeInBothThemes();
    await leaveSettings();
  });

  it('asks for the worker’s change in its task’s card, and gives the conversation the report once allowed', async () => {
    await startNewConversation();
    const instruction = callMessage('write_file', {
      activity: 'Writing the notes',
      content: 'made on the worker',
      path: 'notes.md',
    });
    await send(delegate('Write the notes', instruction));
    await waitForToolStatus('awaiting_approval');

    const shown = $(card(1));
    await waitForShownText(shown, expect.stringContaining('Task 1'));
    await waitForShownText(shown, expect.stringContaining('Write the notes'));
    // The badge of the Workers plugin's kind of task says to screen readers
    // that the name is the worker's. The request names the host as the card
    // does, by the owner's name.
    await waitForShownText($(`${card(1)} [data-task-target][data-task-kind="workers"] [data-content]`), OWNER_NAME);
    await waitForShownText($(`${card(1)} [data-approval-host]`), OWNER_NAME);
    // The request leaves the focus in the message box.
    expect(await focusedElement()).toEqual(expect.objectContaining({ tag: 'textarea' }));
    await axeInBothThemes();

    await clickInLatestTurn('Allow once');

    expect(await waitForCompleteReply()).toContain(
      `${TOOLS_DONE} Last result: Task 1 reported. The evidence in this report comes from the worker's own tool calls:`,
    );
    await waitForTaskState(1, 'reported');
    expect(readFileSync(join(worker.allowed, 'notes.md'), 'utf8')).toBe('made on the worker');
  });

  it('opens the worker’s conversation live in a side panel, read-only', async () => {
    const open = $(`${card(1)} [data-task-open]`);
    await open.scrollIntoView({ block: 'center' });
    await open.click();

    await waitForShownText(`${PANELS} [role="tab"][aria-selected="true"]`, /^Task 1:/u);
    const panel = $(`${PANELS} [role="tabpanel"]:not([hidden])`);
    await panel.$('article').waitForDisplayed({ timeout: 10_000 });
    await waitForShownText(panel, expect.stringContaining('belongs to a task on a worker'));
    await waitForShownText(panel, expect.stringContaining('notes.md'));
    await axeInBothThemes();
    await browser.keys(Key.Escape);
  });

  it('stops a task on the worker', async () => {
    await startNewConversation();
    await send(delegate('Wait forever', 'hang'));
    await waitForTaskState(1, 'running');

    const stop = $(`${card(1)} [data-task-stop]`);
    await stop.scrollIntoView({ block: 'center' });
    await stop.click();

    await waitForTaskState(1, 'canceled');
    await waitForShownText($(card(1)), expect.stringContaining('Stopped'));
  });

  it('shows a worker that stops as offline, and as connected once it is back', async () => {
    await worker.stop();
    await waitForWorkerStatus('offline');
    await axeInBothThemes();

    await worker.start();

    await waitForWorkerStatus('connected');
  });

  it('renames a worker from its menu, and an empty name goes back to the name in its configuration', async () => {
    await $(`${PANEL} [data-worker-menu]`).click();
    await $('[data-slot="dropdown-menu-content"]').waitForDisplayed();
    await clickWithText('[data-slot="dropdown-menu-item"]', 'Rename…');
    await $('[data-rename-worker-dialog]').waitForDisplayed();
    const name = $('#rename-worker-name');
    await expect(name).toHaveValue(OWNER_NAME);
    await waitForShownText($('[data-rename-worker-dialog] dd[data-content]'), WORKER_NAME);
    await waitForAnimations();
    await axeInBothThemes();

    await name.click();
    await browser.keys([Key.Ctrl, 'a']);
    await browser.keys(Key.Backspace);
    await browser.keys(Key.Enter);

    await $('[data-rename-worker-dialog]').waitForExist({ reverse: true, timeout: 10_000 });
    await waitForShownText($(`${PANEL} [data-worker-row]`), expect.stringContaining(WORKER_NAME));
  });

  it('removes a worker after asking, which hides the group', async () => {
    const menu = $(`${PANEL} [data-worker-menu]`);
    await menu.click();
    await $('[data-slot="dropdown-menu-content"]').waitForDisplayed();
    await clickWithText('[data-slot="dropdown-menu-item"]', 'Remove…');
    await $('[data-remove-worker-confirm]').waitForDisplayed();
    await waitForAnimations();
    await axeInBothThemes();

    await $('[data-remove-worker-confirm]').click();

    await browser.waitUntil(
      async () => browser.execute((selector: string) => document.querySelector(selector) === null, PANEL),
      { timeout: 20_000, timeoutMsg: 'the workers group remained after removal' },
    );
    // The worker forgot this app: a new code pairs it again.
    expect(existsSync(join(worker.folder, 'data', 'master-pin'))).toBe(false);
  });

  it('shows only catalog text in the Workers settings in the pseudo-locale', async () => {
    await switchToPseudoLocale();

    await openSettings('workers.settings');

    expect(await uncataloguedText()).toEqual([]);
  });
});
