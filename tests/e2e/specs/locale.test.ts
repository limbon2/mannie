// V11: the message catalogs (ADR-0019). The locale is a setting in the core;
// changing it reloads the window; in the pseudo-locale every visible string is
// marked, the layout survives the longer text, and a hard-coded string is
// caught.
import { Key } from 'webdriverio';

import { axeViolations, uncataloguedText, waitForApp } from '../support/page.ts';

const PSEUDO_LOCALE = 'en-XA';

async function documentLanguage(): Promise<string> {
  return browser.execute(() => document.documentElement.lang);
}

/** Chooses a language in the settings and waits until the window reloaded into it. */
async function chooseLanguage(value: string, expectedLanguage: string): Promise<void> {
  await browser.keys([Key.Ctrl, ',']);
  await $('#interface-language').click();
  await $(`[data-slot="select-item"][data-value="${value}"]`).click();
  await browser.waitUntil(async () => (await documentLanguage()) === expectedLanguage, { timeout: 15_000 });
  await waitForApp();
}

async function overflowsHorizontally(): Promise<boolean> {
  return browser.execute(() => document.documentElement.scrollWidth > document.documentElement.clientWidth);
}

describe('visible text and the locale (V11)', () => {
  before(async () => {
    await waitForApp();
  });

  it('starts in English, the language of the system', async () => {
    expect(await documentLanguage()).toBe('en');
  });

  it('reloads into the pseudo-locale when the language changes', async () => {
    await chooseLanguage(PSEUDO_LOCALE, PSEUDO_LOCALE);

    expect(await documentLanguage()).toBe(PSEUDO_LOCALE);
  });

  it('shows only catalog text in the settings and the main view', async () => {
    await browser.keys([Key.Ctrl, ',']);
    expect(await uncataloguedText()).toEqual([]);

    await browser.execute(() => {
      location.assign('/');
    });
    await browser.waitUntil(async () => browser.execute(() => location.pathname === '/'), { timeout: 15_000 });
    await waitForApp();

    expect(await uncataloguedText()).toEqual([]);
  });

  it('shows only catalog text in the command palette', async () => {
    await browser.keys([Key.Ctrl, 'k']);
    await $('[data-slot="command-input"]').waitForDisplayed();

    expect(await uncataloguedText()).toEqual([]);

    await browser.keys(Key.Escape);
    await $('[data-slot="command-input"]').waitForDisplayed({ reverse: true });
  });

  it('keeps the layout under the longer text', async () => {
    await browser.keys([Key.Ctrl, ',']);
    await $('#interface-language').waitForDisplayed();

    expect(await overflowsHorizontally()).toBe(false);
    expect(await axeViolations()).toEqual([]);
  });

  it('flags a string that bypasses the catalog', async () => {
    await browser.execute(() => {
      const planted = document.createElement('p');
      planted.id = 'planted';
      planted.textContent = 'Planted hard-coded text';
      document.querySelector('main')?.append(planted);
    });

    expect(await uncataloguedText()).toEqual(['Planted hard-coded text']);

    await browser.execute(() => document.querySelector('#planted')?.remove());
  });

  it('keeps the chosen locale across a restart', async () => {
    await browser.reloadSession();
    await waitForApp();

    expect(await documentLanguage()).toBe(PSEUDO_LOCALE);
  });

  it('returns to the system language', async () => {
    await chooseLanguage('*', 'en');

    expect(await documentLanguage()).toBe('en');
  });
});
