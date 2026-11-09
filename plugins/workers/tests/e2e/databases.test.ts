// Databases in the app (plugins/workers/docs/databases.md, acceptance), with
// a real worker on this computer: the feature turns on under Workers on the
// Plugins page and adds its view to the activity bar; the owner registers a
// SQLite file on this computer through the system's file dialog under a
// name, the sidebar lists its tables under it, and its page pages its rows
// and shows a table's structure; a worker whose
// configuration allows databases shows as a group of its own, the owner
// registers a file on it by its path, and its page reads the worker's file
// through the link; a worker that stops leaves its databases listed as
// offline. The database on this computer is added and opened from the
// keyboard, and every step passes axe in both themes (ADR-0055, ADR-0056,
// ADR-0057).
import { mkdtempSync, realpathSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { DatabaseSync } from 'node:sqlite';

import { setFileChoice } from '#e2e/harness.ts';
import {
  axeViolations,
  leaveSettings,
  openSettings,
  runCommand,
  setPluginEnabled,
  waitForApp,
  waitForShownText,
} from '#e2e/page.ts';
import { Key } from 'webdriverio';

import { TestWorker } from './support/worker.ts';

/** The worker's name, which it gives itself in its configuration. */
const WORKER_NAME = 'e2e-databases';

/** The switch of the feature Databases, under Workers on the Plugins page. */
const DATABASES_FEATURE = '[data-plugin-features="workers"] [data-plugin-feature="databases"]';

/** The group of this computer's databases in the view's sidebar. */
const THIS_COMPUTER = '[data-databases-panel][data-databases-machine="this-computer"]';

/** The group of the worker's databases in the view's sidebar. */
const ON_WORKER = '[data-databases-panel][data-databases-machine]:not([data-databases-machine="this-computer"])';

/** The page of the database that the view shows. */
const PAGE = '[data-database-page]';

/** The grid of rows on the page's Data tab. */
const GRID = `${PAGE} [data-database-grid]`;

/** A SQLite file with a table of parts, as an application of the owner would keep it. */
function createDatabase(path: string): void {
  const database = new DatabaseSync(path);
  database.exec(
    "CREATE TABLE parts (name TEXT PRIMARY KEY, stock INTEGER NOT NULL); INSERT INTO parts (name, stock) VALUES ('bolt', 12), ('nut', 40);",
  );
  database.close();
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

/** Moves the keyboard focus to the element that `selector` finds. */
async function focus(selector: string): Promise<void> {
  await $(selector).waitForDisplayed();
  await browser.execute((target: string) => {
    document.querySelector<HTMLElement>(target)?.focus();
  }, selector);
}

/** Types into a field of the open dialog, after clearing it. */
async function fill(selector: string, text: string): Promise<void> {
  const field = $(selector);
  await field.click();
  await field.clearValue();
  await field.addValue(text);
}

describe('databases', () => {
  let folder: string;
  let worker: TestWorker;

  before(async () => {
    // The folder as the system names it, which is how the core compares paths.
    folder = realpathSync.native(mkdtempSync(join(tmpdir(), 'mannie-e2e-databases-')));
    createDatabase(join(folder, 'inventory.db'));
    worker = await TestWorker.start(WORKER_NAME, { databases: true });
    createDatabase(worker.databasePath('parts.db'));
    await waitForApp();
    await setPluginEnabled('workers', true);
  });

  after(async () => {
    await worker.stop();
    worker.remove();
    rmSync(folder, { force: true, maxRetries: 20, recursive: true, retryDelay: 250 });
  });

  it('turns on under Workers on the Plugins page and adds its view to the activity bar', async () => {
    await openSettings('plugins.settings');
    await expect($(DATABASES_FEATURE)).toHaveAttribute('data-plugin-feature-state', 'off');

    const featureSwitch = $(`${DATABASES_FEATURE} [data-plugin-feature-switch]`);
    await featureSwitch.waitForEnabled();
    await featureSwitch.click();

    await $(`${DATABASES_FEATURE}[data-plugin-feature-state="on"]`).waitForExist({
      timeout: 30_000,
      timeoutMsg: 'Databases did not turn on',
    });
    await axeInBothThemes();
    await leaveSettings();
    await runCommand('Show Databases');
    await $(THIS_COMPUTER).waitForDisplayed();
  });

  it('registers a file on this computer through the file dialog and shows its tables and rows', async () => {
    await setFileChoice([join(folder, 'inventory.db')]);
    await focus(`${THIS_COMPUTER} [data-add-database]`);
    await browser.keys(Key.Enter);
    await $('[data-add-database-dialog]').waitForDisplayed();
    await expect($('#add-database-name')).toBeFocused();
    await browser.keys('Inventory'.split(''));

    await browser.keys(Key.Enter);

    await $(PAGE).waitForExist({ timeout: 30_000, timeoutMsg: 'the database did not open' });
    await waitForShownText(PAGE, /inventory\.db on this computer/u);
    await $(`${THIS_COMPUTER} [data-database-table="parts"]`).waitForDisplayed();
    await expect($(`${THIS_COMPUTER} [data-database-row]`)).toHaveText(expect.stringContaining('Inventory'));
    // The page opens on the Data tab with the first table's rows.
    await expect($('[data-database-tab="data"]')).toHaveAttribute('aria-selected', 'true');
    await waitForShownText(GRID, /bolt/u);
    // The tabs follow the arrow keys: Data, then Structure.
    await focus('[data-database-tab="data"]');
    await browser.keys([Key.ArrowRight]);
    await expect($('[data-database-tab="structure"]')).toHaveAttribute('aria-selected', 'true');
    await waitForShownText(`${PAGE} [data-database-structure]`, /stock/u);
    await axeInBothThemes();
  });

  it('registers a file on the worker by its path and reads it through the link', async () => {
    await runCommand('Pair a worker…');
    await $('[data-pair-worker-dialog]').waitForDisplayed();
    await fill('#pair-worker-address', worker.address);
    await fill('#pair-worker-code', worker.code());
    await browser.keys(Key.Enter);
    await $('[data-pair-worker-dialog]').waitForDisplayed({ reverse: true, timeout: 30_000 });
    await runCommand('Show Databases');
    const add = $(`${ON_WORKER} [data-add-database]`);
    await add.waitForDisplayed({ timeout: 30_000, timeoutMsg: 'the worker did not offer databases' });
    await add.click();
    await $('[data-add-database-dialog]').waitForDisplayed();
    await expect($('#add-database-path')).toBeDisplayed();
    await fill('#add-database-path', worker.databasePath('parts.db'));
    await fill('#add-database-name', 'Parts');

    await browser.keys(Key.Enter);

    await $(PAGE).waitForExist({ timeout: 30_000, timeoutMsg: 'the worker’s database did not open' });
    await waitForShownText(PAGE, new RegExp(`parts\\.db on ${WORKER_NAME}`, 'u'));
    await $(`${ON_WORKER} [data-database-table="parts"]`).waitForDisplayed();
    await expect($(`${ON_WORKER} [data-database-row]`)).toHaveText(expect.stringContaining('Parts'));
    await $(`${ON_WORKER} [data-database-table="parts"] button`).click();
    await expect($('[data-database-tab="data"]')).toHaveAttribute('aria-selected', 'true');
    await waitForShownText(GRID, /nut/u);
    await axeInBothThemes();
  });

  it('leaves the databases of a worker that stops listed as offline', async () => {
    await worker.stop();

    // The master notices the dropped link and shows the worker's databases
    // as offline, without the file states that it last listed changing.
    await browser.waitUntil(async () => (await $(PAGE).getText()).includes('Worker offline'), {
      timeout: 60_000,
      timeoutMsg: 'the worker’s database did not show as offline',
    });
    await expect($(`${ON_WORKER} [data-database-row]`)).toHaveText(expect.stringContaining('Parts'));
    await axeInBothThemes();
  });
});
