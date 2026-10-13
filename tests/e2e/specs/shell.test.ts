// The desktop shell (M0): it opens into the working view, works from the
// keyboard and passes the WCAG 2.1 AA checks in both themes (NFR-UX-01,
// NFR-UX-03).
import { Key } from 'webdriverio';

import {
  axeViolations,
  focusedElement,
  focusIsVisible,
  isDark,
  waitForApp,
  waitForShownText,
} from '../support/page.ts';

async function runCommand(label: string): Promise<void> {
  await browser.keys([Key.Ctrl, 'k']);
  const input = await $('[data-slot="command-input"]');
  await input.waitForDisplayed();
  await browser.keys(label.split(''));
  await browser.keys(Key.Enter);
  await $('[data-slot="command-input"]').waitForDisplayed({ reverse: true });
}

describe('the shell', () => {
  before(async () => {
    await waitForApp();
  });

  it('opens into the working view, with the sidebar and the empty main area', async () => {
    await expect($('[data-slot="sidebar"]')).toBeDisplayed();
    await expect($('main#main')).toHaveText(expect.stringContaining('Nothing to show'));
    await expect($('[data-slot="sidebar"]')).toHaveText(expect.stringContaining('No views yet.'));
  });

  it('passes the WCAG 2.1 AA checks in the light theme', async () => {
    await runCommand('Use the light theme');
    expect(await isDark()).toBe(false);
    expect(await axeViolations()).toEqual([]);
  });

  it('passes the WCAG 2.1 AA checks in the dark theme', async () => {
    await runCommand('Use the dark theme');
    expect(await isDark()).toBe(true);
    expect(await axeViolations()).toEqual([]);
  });

  it('passes the WCAG 2.1 AA checks with the command palette open', async () => {
    await browser.keys([Key.Ctrl, 'k']);
    await $('[data-slot="command-input"]').waitForDisplayed();

    expect(await axeViolations()).toEqual([]);

    await browser.keys(Key.Escape);
    await $('[data-slot="command-input"]').waitForDisplayed({ reverse: true });
  });

  it('reaches the skip link first, with a visible focus indicator', async () => {
    await browser.execute(() => {
      (document.activeElement as HTMLElement | null)?.blur();
    });
    await browser.keys(Key.Tab);
    expect(await focusedElement()).toEqual({ label: 'Skip to the main content', tag: 'a' });
    expect(await focusIsVisible()).toBe(true);
  });

  it('opens the settings with Ctrl+, and changes the theme from the keyboard', async () => {
    await browser.keys([Key.Ctrl, ',']);
    await waitForShownText('h1', 'Settings');
    const system = await $('#interface-theme-system');
    await system.click();
    await browser.keys(Key.ArrowUp);
    expect(await isDark()).toBe(true);
    expect(await axeViolations()).toEqual([]);
  });

  it('shows and hides the sidebar with Ctrl+B', async () => {
    const sidebar = await $('[data-slot="sidebar"][data-state]');
    await expect(sidebar).toHaveAttribute('data-state', 'expanded');
    await browser.keys([Key.Ctrl, 'b']);
    await expect(sidebar).toHaveAttribute('data-state', 'collapsed');
    await browser.keys([Key.Ctrl, 'b']);
    await expect(sidebar).toHaveAttribute('data-state', 'expanded');
  });
});
