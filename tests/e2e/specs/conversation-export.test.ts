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

/** Open a seeded conversation without depending on a provider or credentials. */
async function openConversation(id: string): Promise<void> {
  const url = new URL(await browser.getUrl());
  url.pathname = '/';
  url.search = new URLSearchParams({ conversation: id, view: 'chat' }).toString();
  await browser.url(url.toString());
  await waitForApp();
  await $('[data-export-conversation]').waitForClickable();
}

async function savedFolder(): Promise<string> {
  const destination = $('[data-export-destination]');
  await destination.waitForDisplayed();
  await waitForAnimations();
  return shownText(await destination);
}

/** Remove only the temporary directory created for this suite. */
function removeExportFolder(folder: string): void {
  const isInTemporaryDirectory = dirname(folder) === realpathSync(tmpdir());
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
    destination = realpathSync(mkdtempSync(join(tmpdir(), 'mannie-export-')));
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
    await $('[data-export-conversation]').click();
    await $('[data-export-conversation]').waitForEnabled();
    expect(readdirSync(destination)).toEqual([]);
    await expect($('[data-export-destination]')).not.toBeExisting();
  });

  it('exports all persisted turns and supports keyboard access in both themes', async () => {
    await setFolderChoice(destination);
    await browser.execute(() => document.querySelector<HTMLElement>('[data-export-conversation]')?.focus());
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
    await $('[data-export-conversation]').click();
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
    const action = $(`[data-export-conversation="${other}"]`);
    await action.waitForClickable();
    await setFolderChoice(destination);
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
    await $('[data-export-conversation]').click();
    await savedFolder();
    expect(await uncataloguedText()).toEqual([]);
    expect(await axeViolations()).toEqual([]);
    await browser.keys(Key.Escape);
  });
});
