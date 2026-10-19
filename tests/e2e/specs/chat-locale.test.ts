// The chat and window views in the pseudo-locale (ADR-0019; V11 for M1): every
// string they show comes from the catalog, apart from the owner's and the
// model's content, and the layout survives the longer text. The selectors use
// structure only, because every catalog string is pseudo-localized.
import { mkdtempSync, realpathSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { Key } from 'webdriverio';

import {
  addProfile,
  addProject,
  callMessage,
  clickWithText,
  COMPOSER,
  LATEST_MAIN_TURN,
  projectToggle,
  sendMessage,
  startNewConversation,
  waitForCompleteReply,
  waitForPointerInput,
} from '../support/chat.ts';
import { closeMainWindow, deleteProviderProfiles, setFolderChoice } from '../support/harness.ts';
import {
  axeViolations,
  openSettings,
  setPluginEnabled,
  switchToPseudoLocale,
  uncataloguedText,
  waitForAnimations,
  waitForApp,
} from '../support/page.ts';
import { MODELS, type ProviderServer, startProviderServer } from '../support/provider-server.ts';

async function documentLanguage(): Promise<string> {
  return browser.execute(() => document.documentElement.lang);
}

async function overflowsHorizontally(): Promise<boolean> {
  return browser.execute(() => document.documentElement.scrollWidth > document.documentElement.clientWidth);
}

/**
 * Opens the menu of the toolbar's trigger at `index`: the profile, the
 * context or the conversation's actions.
 */
async function openToolbarMenu(index: number): Promise<void> {
  await waitForPointerInput();
  const triggers = await $$('main button[aria-haspopup="menu"]');
  await triggers[index]?.click();
  await $('[data-slot="dropdown-menu-content"]').waitForDisplayed();
  await waitForAnimations();
}

async function closeOverlays(): Promise<void> {
  await browser.keys(Key.Escape);
  await browser.waitUntil(
    async () =>
      browser.execute(
        () =>
          document.querySelector('[data-slot="dropdown-menu-content"], [role="dialog"], [role="alertdialog"]') === null,
      ),
    { timeoutMsg: 'a menu or dialog stayed open' },
  );
  await waitForAnimations();
}

async function openInterfaceSettings(): Promise<void> {
  await openSettings('shell.interface');
  await $('#interface-language').waitForDisplayed();
}

describe('the chat and window views in the pseudo-locale', () => {
  let provider: ProviderServer;

  before(async () => {
    provider = await startProviderServer();
    await waitForApp();
    await addProfile(provider.baseUrl, { model: MODELS.reasoning, name: 'Profile', interface: 'responses' });
    await addProfile(provider.baseUrl, { model: MODELS.tools, name: 'Tools', useTools: true });
    await switchToPseudoLocale();
  });

  after(async () => {
    await deleteProviderProfiles();
    await provider.close();
  });

  it('shows only catalog text in a conversation with a reply', async () => {
    // The window reloaded into the settings, which have a sidebar of their own.
    await startNewConversation();
    await sendMessage('A message in any language');
    await waitForCompleteReply();

    expect(await uncataloguedText()).toEqual([]);
    expect(await overflowsHorizontally()).toBe(false);
    expect(await axeViolations()).toEqual([]);
  });

  it('shows only catalog text in the conversation toolbar menus', async () => {
    for (const index of [0, 1, 2]) {
      await openToolbarMenu(index);
      expect(await uncataloguedText()).toEqual([]);
      await closeOverlays();
    }
  });

  it('shows only catalog text in the rename and delete dialogs', async () => {
    for (const item of [0, 2]) {
      await openToolbarMenu(2);
      const items = await $$('[data-slot="dropdown-menu-item"]');
      await items[item]?.click();
      await $('[role="dialog"], [role="alertdialog"]').waitForDisplayed();
      await waitForAnimations();
      expect(await uncataloguedText()).toEqual([]);
      expect(await overflowsHorizontally()).toBe(false);
      await closeOverlays();
    }
  });

  it('shows only catalog text in a tool call that waits for approval, and in its details', async () => {
    // The profile menu's items show the profiles' names, which are content.
    await openToolbarMenu(0);
    await clickWithText('[data-slot="dropdown-menu-radio-item"]', 'Tools');
    await waitForPointerInput();
    // Without a workspace, a change asks the owner.
    const path = join(tmpdir(), 'mannie-e2e-pseudo-locale.txt');
    // The activity is the model's content, outside the catalog.
    await sendMessage(`call write_file ${JSON.stringify({ activity: 'Writing a note', content: 'x', path })}`);
    const waiting = await $('main [data-tool-status="awaiting_approval"]');
    await waiting.waitForDisplayed({ timeout: 30_000 });
    await $('main [data-tool-status="awaiting_approval"] [data-tool-toggle]').click();
    await waitForAnimations();

    expect(await uncataloguedText()).toEqual([]);
    expect(await overflowsHorizontally()).toBe(false);
    expect(await axeViolations()).toEqual([]);

    // The scopes of "Allow for…".
    await $('main [data-allow-for]').click();
    await $('[data-slot="dropdown-menu-content"]').waitForDisplayed();
    await waitForAnimations();
    expect(await uncataloguedText()).toEqual([]);
    await closeOverlays();

    // Stopping the reply cancels the request.
    await $(COMPOSER).click();
    await browser.keys(Key.Escape);
    await $('main [data-tool-status="canceled"]').waitForDisplayed({ timeout: 10_000 });
    expect(await uncataloguedText()).toEqual([]);
  });

  it('shows only catalog text in a task’s card, its child’s request and the child’s side panel', async () => {
    // The previous test left the Tools profile on. Without a workspace, the
    // child's change asks the owner, in its task's card.
    const path = join(tmpdir(), 'mannie-e2e-pseudo-locale-task.txt');
    const instruction = callMessage('write_file', { activity: 'Writing a note', content: 'x', path });
    await sendMessage(
      callMessage('start_task', { agent: 'general', background: false, instruction, title: 'Write a note' }),
    );
    const card = `${LATEST_MAIN_TURN} [data-task-number="1"]`;
    await $(`${card} [data-tool-status="awaiting_approval"]`).waitForDisplayed({ timeout: 30_000 });
    await $(`${card} [data-task-details]`).click();
    await waitForAnimations();

    expect(await uncataloguedText()).toEqual([]);
    expect(await overflowsHorizontally()).toBe(false);
    expect(await axeViolations()).toEqual([]);

    await $(`${card} [data-task-open]`).click();
    await $('[data-side-panels] article').waitForDisplayed({ timeout: 10_000 });
    await waitForAnimations();
    expect(await uncataloguedText()).toEqual([]);
    expect(await overflowsHorizontally()).toBe(false);
    expect(await axeViolations()).toEqual([]);

    // The toolbar's button lists the conversation's child agents.
    await waitForPointerInput();
    await $('main [data-child-agents]').click();
    await $('[data-slot="dropdown-menu-content"]').waitForDisplayed();
    await waitForAnimations();
    expect(await uncataloguedText()).toEqual([]);
    await closeOverlays();

    // Stopping the parent's reply stops its child.
    await $(COMPOSER).click();
    await browser.keys(Key.Escape);
    await $(`${card}[data-task-state="canceled"]`).waitForDisplayed({ timeout: 10_000 });
    expect(await uncataloguedText()).toEqual([]);
    await $('[data-side-panel-close]').click();
    await $('[data-side-panels]').waitForExist({ reverse: true });
  });

  it('shows only catalog text in the projects of the sidebar, their menu and their dialogs', async () => {
    const folder = realpathSync.native(mkdtempSync(join(tmpdir(), 'mannie-e2e-locale-')));
    try {
      const project = await addProject(folder);
      await waitForAnimations();
      expect(await uncataloguedText()).toEqual([]);
      expect(await overflowsHorizontally()).toBe(false);

      await waitForPointerInput();
      await projectToggle(project).moveTo();
      await $(`[data-project="${project}"] [data-project-menu]`).click();
      await $('[data-slot="dropdown-menu-content"]').waitForDisplayed();
      await waitForAnimations();
      expect(await uncataloguedText()).toEqual([]);
      for (const item of [0, 1]) {
        if (item > 0) {
          await projectToggle(project).moveTo();
          await $(`[data-project="${project}"] [data-project-menu]`).click();
          await $('[data-slot="dropdown-menu-content"]').waitForDisplayed();
        }
        const items = await $$('[data-slot="dropdown-menu-item"]');
        await items[item]?.click();
        await $('[role="dialog"], [role="alertdialog"]').waitForDisplayed();
        await waitForAnimations();
        expect(await uncataloguedText()).toEqual([]);
        expect(await overflowsHorizontally()).toBe(false);
        await closeOverlays();
      }

      // A folder that Mannie refuses opens the dialog for a typed folder, with the cause.
      await setFolderChoice('relative-folder');
      await $('[data-add-project]').click();
      await $('[role="dialog"]').waitForDisplayed();
      await waitForAnimations();
      expect(await uncataloguedText()).toEqual([]);
      expect(await axeViolations()).toEqual([]);
      await closeOverlays();
    } finally {
      rmSync(folder, { force: true, recursive: true });
    }
  });

  it('shows only catalog text on every page of the settings', async () => {
    // A plugin's settings show while the plugin is on; the fixture plugin
    // stands for the plugins, whose own specs check their pages.
    await setPluginEnabled('example', true);
    for (const section of [
      'shell.interface',
      'chat.providers',
      'chat.instructions',
      'window.behavior',
      'tools.settings',
      'child-agents.settings',
      'plugins.settings',
      'example.settings',
    ]) {
      await openSettings(section);

      expect(await uncataloguedText()).toEqual([]);
      expect(await overflowsHorizontally()).toBe(false);
    }
  });

  it('shows only catalog text in the provider dialogs', async () => {
    await openSettings('chat.providers');
    // The first profile row's button without a menu runs the provider test.
    const testButtons = await $$('main li button:not([aria-haspopup])');
    await testButtons[0]?.click();
    await $('[role="dialog"] ul').waitForDisplayed({ timeout: 20_000 });
    await waitForAnimations();
    expect(await uncataloguedText()).toEqual([]);
    await closeOverlays();

    const menuButtons = await $$('main li button[aria-haspopup="menu"]');
    await menuButtons[0]?.click();
    await $('[data-slot="dropdown-menu-content"]').waitForDisplayed();
    await waitForAnimations();
    expect(await uncataloguedText()).toEqual([]);
    const items = await $$('[data-slot="dropdown-menu-item"]');
    await items[0]?.click();
    await $('#profile-name').waitForDisplayed();
    await waitForAnimations();
    expect(await uncataloguedText()).toEqual([]);
    await closeOverlays();
  });

  it('shows only catalog text in the question about closing the window', async () => {
    await closeMainWindow();
    await $('[role="dialog"]').waitForDisplayed();
    await waitForAnimations();

    expect(await uncataloguedText()).toEqual([]);
    expect(await overflowsHorizontally()).toBe(false);
    await closeOverlays();
  });

  it('returns to the system language', async () => {
    await openInterfaceSettings();
    await $('#interface-language').click();
    await $('[data-slot="select-item"][data-value="*"]').click();
    await browser.waitUntil(async () => (await documentLanguage()) === 'en', { timeout: 15_000 });
    await waitForApp();
  });
});
