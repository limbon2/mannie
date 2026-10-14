// The chat and window views in the pseudo-locale (ADR-0019; V11 for M1): every
// string they show comes from the catalog, apart from the owner's and the
// model's content, and the layout survives the longer text. The selectors use
// structure only, because every catalog string is pseudo-localized.
import { Key } from 'webdriverio';

import {
  addProfile,
  sendMessage,
  startNewConversation,
  waitForCompleteReply,
  waitForPointerInput,
} from '../support/chat.ts';
import { closeMainWindow, deleteProviderProfiles } from '../support/harness.ts';
import { axeViolations, openSettings, uncataloguedText, waitForAnimations, waitForApp } from '../support/page.ts';
import { MODELS, type ProviderServer, startProviderServer } from '../support/provider-server.ts';

const PSEUDO_LOCALE = 'en-XA';

async function documentLanguage(): Promise<string> {
  return browser.execute(() => document.documentElement.lang);
}

async function overflowsHorizontally(): Promise<boolean> {
  return browser.execute(() => document.documentElement.scrollWidth > document.documentElement.clientWidth);
}

/** Opens the menu of the toolbar's trigger at `index`: the profile, the context or the conversation's actions. */
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
    await openInterfaceSettings();
    await $('#interface-language').click();
    await $(`[data-slot="select-item"][data-value="${PSEUDO_LOCALE}"]`).click();
    await browser.waitUntil(async () => (await documentLanguage()) === PSEUDO_LOCALE, { timeout: 15_000 });
    await waitForApp();
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

  it('shows only catalog text on every page of the settings', async () => {
    for (const section of ['shell.interface', 'chat.providers', 'chat.instructions', 'window.behavior']) {
      await openSettings(section);

      expect(await uncataloguedText()).toEqual([]);
      expect(await overflowsHorizontally()).toBe(false);
    }
  });

  it('shows only catalog text in the provider dialogs', async () => {
    await openSettings('chat.providers');
    // The profile row's button without a menu runs the provider test.
    await $('main li button:not([aria-haspopup])').click();
    await $('[role="dialog"] ul').waitForDisplayed({ timeout: 20_000 });
    await waitForAnimations();
    expect(await uncataloguedText()).toEqual([]);
    await closeOverlays();

    await $('main li button[aria-haspopup="menu"]').click();
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
