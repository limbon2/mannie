import { mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join } from 'node:path';

import { Key } from 'webdriverio';

import { seedConversation, setFolderChoice } from '../support/harness.ts';
import {
  axeViolations,
  openSettings,
  shownText,
  switchToPseudoLocale,
  uncataloguedText,
  waitForAnimations,
  waitForApp,
  waitForShownText,
} from '../support/page.ts';

/** The button of the conversation's menu in the main view, which holds the export (ADR-0042). */
const ACTIONS = 'main [data-conversation-actions]';

/** The export's item in an open conversation menu. */
const EXPORT_ITEM = '[data-slot="dropdown-menu-content"] [data-export-conversation]';

/** Open a seeded conversation without depending on a provider or credentials. */
async function openConversation(id: string): Promise<void> {
  const url = new URL(await browser.getUrl());
  url.pathname = '/';
  url.search = new URLSearchParams({ conversation: id, view: 'chat' }).toString();
  await browser.url(url.toString());
  await waitForApp();
  await $(ACTIONS).waitForClickable();
}

/** Chooses "Export conversation…" in the conversation menu whose button matches `actions`. */
async function chooseExport(actions = ACTIONS): Promise<void> {
  await $(actions).click();
  const item = $(EXPORT_ITEM);
  await item.waitForClickable();
  await item.click();
}

async function savedFolder(): Promise<string> {
  const destination = $('[data-export-destination]');
  await destination.waitForDisplayed();
  await waitForAnimations();
  return shownText(await destination);
}

/** Remove only the temporary directory created for this suite. */
function removeExportFolder(folder: string): void {
  const isInTemporaryDirectory = dirname(folder) === realpathSync.native(tmpdir());
  const isExportTestFolder = basename(folder).startsWith('mannie-export-');
  if (!isInTemporaryDirectory || !isExportTestFolder) {
    throw new Error('unexpected export test folder');
  }
  rmSync(folder, { force: true, recursive: true, maxRetries: 20, retryDelay: 250 });
}

describe('conversation export', () => {
  let destination: string;
  let conversation: string;

  before(async () => {
    destination = realpathSync.native(mkdtempSync(join(tmpdir(), 'mannie-export-')));
    await waitForApp();
    conversation = await seedConversation(60);
    await openConversation(conversation);
  });

  after(async () => {
    try {
      await openSettings('shell.interface');
      await $('#interface-language').click();
      await $('[data-slot="select-item"][data-value="*"]').click();
      await browser.waitUntil(async () => (await browser.execute(() => document.documentElement.lang)) === 'en');
    } finally {
      removeExportFolder(destination);
    }
  });

  it('cancels without creating an export folder', async () => {
    await setFolderChoice(null);
    await chooseExport();
    // The menu offers the export again once the canceled one ended.
    await $(ACTIONS).click();
    await browser.waitUntil(async () => (await $(EXPORT_ITEM).getAttribute('data-disabled')) === null, {
      timeoutMsg: 'the export did not end after the folder choice was canceled',
    });
    await browser.keys(Key.Escape);
    expect(readdirSync(destination)).toEqual([]);
    await expect($('[data-export-destination]')).not.toBeExisting();
  });

  it('exports all persisted turns and supports keyboard access in both themes', async () => {
    await setFolderChoice(destination);
    await browser.execute((actions: string) => document.querySelector<HTMLElement>(actions)?.focus(), ACTIONS);
    await browser.keys(Key.Enter);
    // The menu opens from the keyboard with its first item, the export, focused.
    await $(EXPORT_ITEM).waitForDisplayed();
    await browser.keys(Key.Enter);
    const folder = await savedFolder();
    const markdown = readFileSync(join(folder, 'conversation.md'), 'utf8');
    expect(markdown.match(/^## Turn /gm)).toHaveLength(60);
    expect(readdirSync(folder)).not.toContain('.incomplete');
    expect(realpathSync.native(dirname(folder))).toBe(destination);
    expect(await browser.execute(() => document.activeElement?.closest('[role="dialog"]') !== null)).toBe(true);
    await waitForShownText('[role="dialog"] [role="status"]', 'The conversation was exported.');
    expect(await axeViolations()).toEqual([]);
    await browser.execute(() => document.documentElement.classList.add('dark'));
    expect(await axeViolations()).toEqual([]);
    await browser.execute(() => document.documentElement.classList.remove('dark'));
    await browser.keys(Key.Escape);
  });

  it('exports from the command palette without replacing an earlier export', async () => {
    await setFolderChoice(destination);
    await browser.keys([Key.Ctrl, 'k']);
    const input = $('[data-slot="command-input"]');
    await input.waitForDisplayed();
    await input.setValue('Export conversation');
    await browser.keys(Key.Enter);
    await savedFolder();
    expect(readdirSync(destination)).toHaveLength(2);
    await browser.keys(Key.Escape);
  });

  it('reports an unwritable destination and retries through a new native choice', async () => {
    await setFolderChoice(join(destination, 'missing-parent'));
    await chooseExport();
    await $('[role="dialog"] [data-slot="alert"]').waitForDisplayed();
    await waitForShownText('[role="dialog"]', expect.stringContaining('could not be written'));
    await setFolderChoice(destination);
    await $('[role="dialog"] [data-slot="dialog-footer"] button:first-child').click();
    await savedFolder();
    await browser.keys(Key.Escape);
  });

  it('exports the conversation shown in a side panel', async () => {
    const other = await seedConversation(3);
    const url = new URL(await browser.getUrl());
    url.searchParams.set('panel', `chat.conversation:${other}`);
    url.searchParams.set('shown-panel', `chat.conversation:${other}`);
    await browser.url(url.toString());
    await waitForApp();
    const panelActions = '[data-side-panels] [data-conversation-actions]';
    await $(panelActions).waitForClickable();
    await setFolderChoice(destination);
    await $(panelActions).click();
    const action = $(`[data-export-conversation="${other}"]`);
    await action.waitForClickable();
    await action.click();
    const folder = await savedFolder();
    expect(readFileSync(join(folder, 'conversation.md'), 'utf8').match(/^## Turn /gm)).toHaveLength(3);
    await browser.keys(Key.Escape);
    await openConversation(conversation);
  });

  it('keeps export feedback in the message catalog in the pseudo-locale', async () => {
    await switchToPseudoLocale();
    await openConversation(conversation);
    await setFolderChoice(destination);
    await chooseExport();
    await savedFolder();
    expect(await uncataloguedText()).toEqual([]);
    expect(await axeViolations()).toEqual([]);
    await browser.keys(Key.Escape);
  });
});
