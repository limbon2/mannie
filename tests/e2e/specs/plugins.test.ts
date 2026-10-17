// Plugins in the app (docs/features/plugins.md, behavior): a new installation
// lists Workers off with none of its parts showing; turning it on from the
// keyboard shows its settings page and its command at once, and turning it off
// hides them again, without reloading the window; the address of a settings
// page of a plugin that is off says so and leads to the Plugins page. Every
// page passes axe in both themes.
import { Key } from 'webdriverio';

import { axeViolations, leaveSettings, openSettings, runCommand, waitForApp } from '../support/page.ts';

/** The Workers plugin's row on the Plugins page. */
const WORKERS = '[data-plugin="workers"]';

/** The switch in the Workers plugin's row. */
const WORKERS_SWITCH = `${WORKERS} [data-plugin-switch]`;

/** The settings sidebar's link to the Workers settings. */
const WORKERS_SETTINGS_LINK = '[data-slot="sidebar"] a[href$="?section=workers.settings"]';

/** The settings sidebar's link to the Plugins settings. */
const PLUGINS_SETTINGS_LINK = '[data-slot="sidebar"] a[href$="?section=plugins.settings"]';

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

/** Waits until the Plugins page shows Workers in `state`. */
async function waitForWorkersState(state: string): Promise<void> {
  await $(`${WORKERS}[data-plugin-state="${state}"]`).waitForExist({
    timeout: 30_000,
    timeoutMsg: `Workers did not turn ${state}`,
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

  it('lists Workers off in a new installation, with none of its parts showing', async () => {
    await openSettings('plugins.settings');

    await expect($(WORKERS)).toHaveAttribute('data-plugin-state', 'off');
    await expect($(WORKERS_SWITCH)).toHaveAttribute('aria-checked', 'false');
    await expect($('[data-plugin="images"]')).toHaveAttribute('data-plugin-state', 'off');
    await expect($(WORKERS_SETTINGS_LINK)).not.toBeExisting();
    await axeInBothThemes();
    await leaveSettings();
    expect(await commandLabels()).not.toContain('Pair a worker…');
  });

  it('turns Workers on from the keyboard at once, without reloading the window', async () => {
    await openSettings('plugins.settings');
    await browser.execute((selector: string) => {
      document.querySelector<HTMLElement>(selector)?.focus();
    }, WORKERS_SWITCH);

    await browser.keys(Key.Space);

    await waitForWorkersState('on');
    await expect($(WORKERS_SWITCH)).toHaveAttribute('aria-checked', 'true');
    await $(WORKERS_SETTINGS_LINK).waitForExist();
    await leaveSettings();
    await runCommand('Pair a worker…');
    await $('[data-pair-worker-dialog]').waitForDisplayed();
    await browser.keys(Key.Escape);
    await $('[data-pair-worker-dialog]').waitForDisplayed({ reverse: true });
    expect(await windowIsMarked()).toBe(true);
  });

  it('turns Workers off at once, and then says on its settings page that it is off, with the way to the Plugins page', async () => {
    await openSettings('workers.settings');
    await $(PLUGINS_SETTINGS_LINK).click();
    await $(WORKERS_SWITCH).waitForClickable();

    await $(WORKERS_SWITCH).click();

    await waitForWorkersState('off');
    await expect($(WORKERS_SETTINGS_LINK)).not.toBeExisting();
    // Back to the Workers settings' address, which the app opens without a reload.
    await browser.back();
    const offState = await $('[data-plugin-off="workers"]');
    await offState.waitForDisplayed();
    await axeInBothThemes();
    await offState.$('button').click();
    await browser.waitUntil(async () => browser.execute(() => location.search === '?section=plugins.settings'), {
      timeoutMsg: 'the Plugins settings did not open',
    });
    await leaveSettings();
    expect(await commandLabels()).not.toContain('Pair a worker…');
    expect(await windowIsMarked()).toBe(true);
  });
});
