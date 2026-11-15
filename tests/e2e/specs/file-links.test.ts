// Local links exercise the real IPC path; validation builds record actions.
import { mkdirSync, mkdtempSync, realpathSync, rmSync, unlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

import { Key } from 'webdriverio';

import {
  addProfile,
  addProject,
  callMessage,
  clickWithText,
  sendMessage,
  startConversationIn,
  startNewConversation,
  TEST_CREDENTIAL,
  useProfile,
  waitForCompleteReply,
} from '../support/chat.ts';
import { deleteProviderProfiles, openedFileLinks } from '../support/harness.ts';
import { axeViolations, waitForAnimations, waitForApp } from '../support/page.ts';
import { MODELS, type ProviderServer, startProviderServer } from '../support/provider-server.ts';

const LINK = 'main [data-file-link]';

describe('file links', () => {
  let provider: ProviderServer;
  let directory: string;
  let file: string;
  let source: string;
  let project: string;

  async function actions(count: number): Promise<void> {
    await browser.waitUntil(async () => (await openedFileLinks()).length === count, {
      timeout: 10_000,
      timeoutMsg: 'the file-manager action was not recorded exactly once',
    });
  }

  before(async () => {
    directory = mkdtempSync(join(tmpdir(), 'mannie-file-links-'));
    mkdirSync(join(directory, 'src'));
    file = join(directory, 'file é with spaces.txt');
    source = join(directory, 'src', 'main.rs');
    writeFileSync(file, 'file link fixture');
    writeFileSync(source, 'source fixture');
    writeFileSync(join(directory, 'child.md'), '[Child file](src/main.rs)');
    provider = await startProviderServer();
    provider.setHostileText(
      [
        `[File](${pathToFileURL(file).href})`,
        `[Folder](${pathToFileURL(directory).href})`,
        '\x60src/main.rs:42:7\x60',
        '[Missing](missing.rs)',
        '\x60cargo test src/main.rs\x60',
        '\x60\x60\x60\nsrc/main.rs\n\x60\x60\x60',
      ].join('\n\n'),
    );
    await waitForApp();
    await addProfile(provider.baseUrl, { credential: TEST_CREDENTIAL, model: MODELS.hostile, name: 'File links' });
    project = await addProject(directory);
    await sendMessage('Show the file references');
    await waitForCompleteReply();
  });

  after(async () => {
    await deleteProviderProfiles();
    await provider.close();
    rmSync(directory, { force: true, recursive: true });
  });

  it('renders without opening anything and reveals an explicitly linked file', async () => {
    expect(await openedFileLinks()).toEqual([]);
    expect(await $$(LINK)).toHaveLength(4);
    await clickWithText(LINK, 'File');
    await actions(1);
    expect((await openedFileLinks())[0]).toMatchObject({ path: realpathSync(file), kind: 'file' });
    expect(await $('[role="alertdialog"]').isExisting()).toBe(false);
  });

  it('opens a relative inline path from the keyboard, including line and column suffixes', async () => {
    await browser.execute(() => {
      const folder = [...document.querySelectorAll<HTMLButtonElement>('main [data-file-link]')].find(
        (element) => element.textContent.trim() === 'Folder',
      );
      folder?.focus();
    });
    await browser.keys(Key.Tab);
    await expect($('//main//button[@data-file-link and normalize-space(.)="src/main.rs:42:7"]')).toBeFocused();
    await browser.keys(Key.Enter);
    await actions(2);
    await browser.keys(Key.Space);
    await actions(3);
    expect((await openedFileLinks()).slice(1)).toEqual([
      expect.objectContaining({ path: realpathSync(source), kind: 'file' }),
      expect.objectContaining({ path: realpathSync(source), kind: 'file' }),
    ]);
  });

  it('opens the folder through the right-click submenu and restores focus', async () => {
    const folder = await $('//main//button[@data-file-link and normalize-space(.)="Folder"]');
    await folder.click({ button: 'right' });
    const submenu = await $('[data-slot="context-menu-sub-trigger"]');
    await browser.waitUntil(async () => (await submenu.getAttribute('data-disabled')) === null);
    await submenu.moveTo();
    const item = await $('[data-slot="context-menu-sub-content"] [data-slot="context-menu-item"]');
    await item.waitForDisplayed();
    expect(await item.getText()).toBe(process.platform === 'win32' ? 'File Explorer' : 'File manager');
    expect(await openedFileLinks()).toHaveLength(3);
    await item.click();
    await actions(4);
    expect((await openedFileLinks())[3]).toMatchObject({ path: realpathSync(directory), kind: 'directory' });
    await browser.waitUntil(async () => folder.isFocused(), {
      timeoutMsg: await browser.execute(() => document.activeElement?.outerHTML ?? 'no focus'),
    });
  });

  it('supports the keyboard context menu and submenu', async () => {
    await browser.keys([Key.Shift, Key.F10]);
    const submenu = await $('[data-slot="context-menu-sub-trigger"]');
    await submenu.waitForDisplayed();
    await browser.waitUntil(async () => (await submenu.getAttribute('data-disabled')) === null);
    await browser.keys(Key.ArrowDown);
    await browser.keys(Key.ArrowRight);
    await browser.keys(Key.Enter);
    await actions(5);
    expect((await openedFileLinks())[4]?.kind).toBe('directory');
    await waitForAnimations();
    expect(await axeViolations()).toEqual([]);
    await browser.keys([Key.Shift, Key.F10]);
    await submenu.waitForDisplayed();
    await browser.keys(Key.Escape);
    await browser.waitUntil(async () => (await $$('[data-slot="context-menu-content"]')).length === 0);
    await expect($('//main//button[@data-file-link and normalize-space(.)="Folder"]')).toBeFocused();
    expect(await openedFileLinks()).toHaveLength(5);
  });

  it('reports missing and deleted paths without recording another action', async () => {
    await clickWithText(LINK, 'Missing');
    await browser.waitUntil(async () => (await $('main [role="alert"]').getText()).includes('does not exist'));
    unlinkSync(file);
    await clickWithText(LINK, 'File');
    await browser.waitUntil(async () => (await $$('main [role="alert"]')).length === 2);
    expect(await openedFileLinks()).toHaveLength(5);
  });

  it('never guesses a working directory for a general conversation', async () => {
    await startNewConversation();
    provider.setHostileText('[Relative](src/main.rs)');
    await sendMessage('Show a relative reference without a project');
    await waitForCompleteReply();
    await clickWithText(LINK, 'Relative');
    await browser.waitUntil(async () => (await $('main [role="alert"]').getText()).includes('has no project'));
    expect(await openedFileLinks()).toHaveLength(5);
  });

  it('resolves the child panel’s project beside a different main conversation', async () => {
    const generalAddress = new URL(await browser.getUrl());
    await addProfile(provider.baseUrl, { model: MODELS.tools, name: 'File link tools', useTools: true });
    await startConversationIn(project);
    await useProfile('File link tools');
    await sendMessage(
      callMessage('start_task', {
        agent: 'explore',
        background: false,
        instruction: callMessage('read_file', { path: 'child.md' }),
        title: 'Read the reference',
      }),
    );
    await waitForCompleteReply();
    const open = await $('main [data-task-open]');
    await open.scrollIntoView({ block: 'center' });
    await open.click();
    await browser.waitUntil(async () => (await $$('[data-side-panels] [data-file-link]')).length > 0);
    const panelAddress = new URL(await browser.getUrl());
    for (const parameter of ['panel', 'shown-panel']) {
      for (const value of panelAddress.searchParams.getAll(parameter)) {
        generalAddress.searchParams.append(parameter, value);
      }
    }
    await browser.url(generalAddress.href);
    await waitForApp();
    await clickWithText('[data-side-panels] [data-file-link]', 'Child file');
    await actions(6);
    expect((await openedFileLinks())[5]).toMatchObject({ path: realpathSync(source), kind: 'file' });
    await $('[data-side-panel-close]').click();
  });
});
