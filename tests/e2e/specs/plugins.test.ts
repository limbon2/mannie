// Plugins in the app (docs/features/plugins.md, behavior), with the fixture
// plugin that validation builds bundle in place of the plugins that ship
// (ADR-0035, decision 9): a new installation lists it off with none of its
// parts showing; what it may use opens from the keyboard; turning it on from
// the keyboard shows its settings page and its command at once, and turning
// it off hides them again, without reloading the window; the address of a
// settings page of a plugin that is off says so and leads to the Plugins
// page. Every page passes axe in both themes.
import { Key } from 'webdriverio';

import { axeViolations, leaveSettings, openPluginUses, openSettings, runCommand, waitForApp } from '../support/page.ts';

/** The fixture plugin's row on the Plugins page. */
const EXAMPLE = '[data-plugin="example"]';

/** The switch in the fixture plugin's row. */
const EXAMPLE_SWITCH = `${EXAMPLE} [data-plugin-switch]`;

/** The settings sidebar's link to the fixture plugin's settings. */
const EXAMPLE_SETTINGS_LINK = '[data-slot="sidebar"] a[href$="?section=example.settings"]';

/** The settings sidebar's link to the Plugins settings. */
const PLUGINS_SETTINGS_LINK = '[data-slot="sidebar"] a[href$="?section=plugins.settings"]';

/** The fixture plugin's command in the command palette. */
const EXAMPLE_COMMAND = 'Add an example note…';

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

/** Marks the window, so that a reload, which would drop the mark, shows. */
async function markWindow(): Promise<void> {
  await browser.execute(() => {
    Reflect.set(window, 'mannieE2eMark', true);
  });
}

/** Whether the window still has the mark, so it did not reload. */
async function windowIsMarked(): Promise<boolean> {
  return browser.execute(() => Reflect.get(window, 'mannieE2eMark') === true);
}

/** Waits until the Plugins page shows the fixture plugin in `state`. */
async function waitForExampleState(state: string): Promise<void> {
  await $(`${EXAMPLE}[data-plugin-state="${state}"]`).waitForExist({
    timeout: 30_000,
    timeoutMsg: `the example plugin did not turn ${state}`,
  });
}

/** The labels of the command palette's commands. */
async function commandLabels(): Promise<string[]> {
  await browser.keys([Key.Ctrl, 'k']);
  await $('[data-slot="command-input"]').waitForDisplayed();
  const labels = await browser.execute(() =>
    [...document.querySelectorAll('[data-slot="command-item"]')].map((item) => item.textContent.trim()),
  );
  await browser.keys(Key.Escape);
  await $('[data-slot="command-input"]').waitForDisplayed({ reverse: true });
  return labels;
}

describe('plugins', () => {
  before(async () => {
    await waitForApp();
    await markWindow();
  });

  it('lists the example plugin off in a new installation, with none of its parts showing', async () => {
    await openSettings('plugins.settings');

    await expect($(EXAMPLE)).toHaveAttribute('data-plugin-state', 'off');
    await expect($(EXAMPLE_SWITCH)).toHaveAttribute('aria-checked', 'false');
    await expect($(EXAMPLE_SETTINGS_LINK)).not.toBeExisting();
    await axeInBothThemes();
    await leaveSettings();
    expect(await commandLabels()).not.toContain(EXAMPLE_COMMAND);
  });

  it('lists what a plugin may use: its services, commands and channels', async () => {
    await openSettings('plugins.settings');

    await openPluginUses('example');

    await expect($(`${EXAMPLE} [data-plugin-service="store"]`)).toBeDisplayed();
    await expect($(`${EXAMPLE} [data-plugin-service="store"] [data-plugin-service-sensitive]`)).not.toBeExisting();
    await expect($(`${EXAMPLE} [data-plugin-command="add_note"]`)).toBeDisplayed();
    await expect($(`${EXAMPLE} [data-plugin-channel="follow_notes"]`)).toBeDisplayed();
    await axeInBothThemes();
    await leaveSettings();
  });

  it('turns the example plugin on from the keyboard at once, without reloading the window', async () => {
    await openSettings('plugins.settings');
    await browser.execute((selector: string) => {
      document.querySelector<HTMLElement>(selector)?.focus();
    }, EXAMPLE_SWITCH);

    await browser.keys(Key.Space);

    await waitForExampleState('on');
    await expect($(EXAMPLE_SWITCH)).toHaveAttribute('aria-checked', 'true');
    await $(EXAMPLE_SETTINGS_LINK).waitForExist();
    await leaveSettings();
    await runCommand(EXAMPLE_COMMAND);
    await $('[data-example-settings]').waitForDisplayed();
    expect(await windowIsMarked()).toBe(true);
  });

  it('turns the example plugin off at once, and then says on its settings page that it is off, with the way to the Plugins page', async () => {
    await openSettings('example.settings');
    await $(PLUGINS_SETTINGS_LINK).click();
    await $(EXAMPLE_SWITCH).waitForClickable();

    await $(EXAMPLE_SWITCH).click();

    await waitForExampleState('off');
    await expect($(EXAMPLE_SETTINGS_LINK)).not.toBeExisting();
    // Back to the plugin's settings address, which the app opens without a reload.
    await browser.back();
    const offState = await $('[data-plugin-off="example"]');
    await offState.waitForDisplayed();
    await axeInBothThemes();
    await offState.$('button').click();
    await browser.waitUntil(async () => browser.execute(() => location.search === '?section=plugins.settings'), {
      timeoutMsg: 'the Plugins settings did not open',
    });
    await leaveSettings();
    expect(await commandLabels()).not.toContain(EXAMPLE_COMMAND);
    expect(await windowIsMarked()).toBe(true);
  });
});
