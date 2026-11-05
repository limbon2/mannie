// Plugins in the app (docs/features/plugins.md, behavior), with the fixture
// plugin that validation builds bundle in place of the plugins that ship
// (ADR-0035, decision 9): a new installation lists it off with none of its
// parts showing; what it may use opens from the keyboard; turning it on from
// the keyboard shows its settings page and its command at once, and turning
// it off hides them again, without reloading the window; the address of a
// settings page of a plugin that is off says so and leads to the Plugins
// page. Its optional feature lists under it with a switch that works while
// the plugin is on, shows its settings page only while it is on, keeps the
// owner's choice while the plugin is off, and starts again with the plugin
// (ADR-0055). Every page passes axe in both themes.
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

/** The row of the fixture plugin's optional feature, in the list under the plugin's row. */
const COUNTS = '[data-plugin-features="example"] [data-plugin-feature="counts"]';

/** The switch in the feature's row. */
const COUNTS_SWITCH = `${COUNTS} [data-plugin-feature-switch]`;

/** The settings sidebar's link to the feature's settings. */
const COUNTS_SETTINGS_LINK = '[data-slot="sidebar"] a[href$="?section=example.counts"]';

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

/** Waits until the Plugins page shows the fixture plugin's feature in `state`. */
async function waitForCountsState(state: string): Promise<void> {
  await $(`${COUNTS}[data-plugin-feature-state="${state}"]`).waitForExist({
    timeout: 30_000,
    timeoutMsg: `the example plugin's feature did not turn ${state}`,
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

  it("lists the example plugin's feature under it, off, with a switch that waits for the plugin", async () => {
    await openSettings('plugins.settings');

    await expect($(COUNTS)).toHaveAttribute('data-plugin-feature-state', 'off');
    await expect($(COUNTS_SWITCH)).toHaveAttribute('aria-checked', 'false');
    await expect($(COUNTS_SWITCH)).toBeDisabled();
    await expect($(COUNTS_SETTINGS_LINK)).not.toBeExisting();
    await axeInBothThemes();
    await leaveSettings();
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

  it("turns the plugin's feature on from the keyboard at once, and its settings page counts the notes", async () => {
    await openSettings('plugins.settings');
    await $(COUNTS_SWITCH).waitForEnabled();
    await browser.execute((selector: string) => {
      document.querySelector<HTMLElement>(selector)?.focus();
    }, COUNTS_SWITCH);

    await browser.keys(Key.Space);

    await waitForCountsState('on');
    await expect($(COUNTS_SWITCH)).toHaveAttribute('aria-checked', 'true');
    await $(COUNTS_SETTINGS_LINK).click();
    await $('[data-example-counts] [data-example-count]').waitForDisplayed();
    await axeInBothThemes();
    expect(await windowIsMarked()).toBe(true);
  });

  it('turns the feature off at once, and then says on its settings page that the feature is off', async () => {
    await openSettings('example.counts');
    await $(PLUGINS_SETTINGS_LINK).click();
    await $(COUNTS_SWITCH).waitForClickable();

    await $(COUNTS_SWITCH).click();

    await waitForCountsState('off');
    await expect($(EXAMPLE)).toHaveAttribute('data-plugin-state', 'on');
    await expect($(COUNTS_SETTINGS_LINK)).not.toBeExisting();
    // Back to the feature's settings address, which the app opens without a reload.
    await browser.back();
    const offState = await $('[data-plugin-feature-off="counts"]');
    await offState.waitForDisplayed();
    await axeInBothThemes();
    await offState.$('button').click();
    await browser.waitUntil(async () => browser.execute(() => location.search === '?section=plugins.settings'), {
      timeoutMsg: 'the Plugins settings did not open',
    });
    // On again, so that the plugin's own checks below find it on.
    await $(COUNTS_SWITCH).waitForClickable();
    await $(COUNTS_SWITCH).click();
    await waitForCountsState('on');
    await leaveSettings();
    expect(await windowIsMarked()).toBe(true);
  });

  it('turns the example plugin off at once, and then says on its settings page that it is off, with the way to the Plugins page', async () => {
    await openSettings('example.settings');
    await $(PLUGINS_SETTINGS_LINK).click();
    await $(EXAMPLE_SWITCH).waitForClickable();

    await $(EXAMPLE_SWITCH).click();

    await waitForExampleState('off');
    await expect($(EXAMPLE_SETTINGS_LINK)).not.toBeExisting();
    // The feature stopped with its plugin and keeps the owner's choice.
    await expect($(COUNTS)).toHaveAttribute('data-plugin-feature-state', 'off');
    await expect($(COUNTS_SWITCH)).toHaveAttribute('aria-checked', 'true');
    await expect($(COUNTS_SETTINGS_LINK)).not.toBeExisting();
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

  it('starts the feature again with its plugin, as the owner left it', async () => {
    await openSettings('plugins.settings');
    await $(EXAMPLE_SWITCH).waitForClickable();

    await $(EXAMPLE_SWITCH).click();

    await waitForExampleState('on');
    await waitForCountsState('on');
    await $(COUNTS_SETTINGS_LINK).waitForExist();
    // Off again, as a new installation would have them.
    await $(COUNTS_SWITCH).click();
    await waitForCountsState('off');
    await $(EXAMPLE_SWITCH).click();
    await waitForExampleState('off');
    await leaveSettings();
    expect(await windowIsMarked()).toBe(true);
  });
});
