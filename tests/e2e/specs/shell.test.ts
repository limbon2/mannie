// The desktop shell (M0): it opens into the working view, works from the
// keyboard and passes the WCAG 2.1 AA checks in both themes (NFR-UX-01,
// NFR-UX-03).
import { Key } from 'webdriverio';

import {
  axeViolations,
  focusedElement,
  focusIsVisible,
  isDark,
  leaveSettings,
  openSettings,
  runCommand,
  waitForApp,
  waitForShownText,
} from '../support/page.ts';

describe('the shell', () => {
  before(async () => {
    await waitForApp();
  });

  it('opens into the chat view, with the activity bar and the conversation list in the sidebar', async () => {
    await expect($('[data-slot="sidebar"]')).toBeDisplayed();
    await expect($('main#main textarea[aria-describedby="composer-hint"]')).toBeDisplayed();
    // The activity bar selects Chat, whose sidebar shows the chat feature's
    // panels, the projects and the conversations, and the bar's foot holds
    // the command palette and the settings (ADR-0045, ADR-0023).
    const bar = await browser.execute(() =>
      [...document.querySelectorAll('[data-shell-sidebar="bar"] [data-sidebar="menu-button"]')].map((button) => ({
        current: button.getAttribute('aria-current'),
        text: button.textContent.trim(),
      })),
    );
    expect(bar).toEqual([
      { current: 'page', text: 'Chat' },
      { current: null, text: 'Find a command' },
      { current: null, text: 'Settings' },
    ]);
    await expect($('nav[aria-label="Views"]')).toBeDisplayed();
    const labels = await browser.execute(() =>
      [...document.querySelectorAll('[data-shell-sidebar="content"] [data-slot="sidebar-group-label"]')].map((label) =>
        label.textContent.trim(),
      ),
    );
    expect(labels).toEqual(['Projects', 'Conversations']);
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

  it('shows the settings as a page of their own, with a page per section and a way back', async () => {
    // The previous test left the settings open. A new conversation gives the
    // main view search parameters, which the way back restores.
    await leaveSettings();
    await $('[data-new-conversation]').click();
    await browser.waitUntil(async () => browser.execute(() => location.search === '?view=chat'), {
      timeoutMsg: 'the new conversation did not open',
    });

    await openSettings('window.behavior');

    const entries = await browser.execute(() =>
      [...document.querySelectorAll('[data-shell-sidebar="content"] [data-sidebar="menu-button"]')].map((entry) =>
        entry.textContent.trim(),
      ),
    );
    expect(entries).toEqual([
      'Back',
      'Interface',
      'Providers',
      'Instructions',
      'Tools',
      'Child agents',
      'Window',
      'Plugins',
    ]);
    await waitForShownText('[id="settings-section-window.behavior"]', 'Window');
    await expect($('#interface-theme-system')).not.toBeExisting();
    await expect($('main#main textarea[aria-describedby="composer-hint"]')).not.toBeExisting();
    expect(await axeViolations()).toEqual([]);

    await leaveSettings();

    await expect($('main#main textarea[aria-describedby="composer-hint"]')).toBeDisplayed();
    expect(await browser.execute(() => location.search)).toBe('?view=chat');
  });

  it('shows and hides the sidebar with Ctrl+B, and keeps the activity bar', async () => {
    const sidebar = await $('[data-slot="sidebar"][data-state]');
    await expect(sidebar).toHaveAttribute('data-state', 'expanded');
    await browser.keys([Key.Ctrl, 'b']);
    await expect(sidebar).toHaveAttribute('data-state', 'collapsed');
    // The content leaves the page, so the focus cannot reach it, and the bar stays.
    await expect($('[data-shell-sidebar="content"]')).not.toBeDisplayed();
    await expect($('nav[aria-label="Views"]')).toBeDisplayed();
    expect(await axeViolations()).toEqual([]);
    await browser.keys([Key.Ctrl, 'b']);
    await expect(sidebar).toHaveAttribute('data-state', 'expanded');
  });

  it('shows each entry of the activity bar from the command palette', async () => {
    await runCommand('Show Chat');
    // The bar shows only icons, so the entry's label is its accessible name
    // and its tooltip, which the visible text does not show.
    const current = await browser.execute(
      () => document.querySelector('[data-shell-sidebar="bar"] a[aria-current="page"]')?.textContent.trim() ?? null,
    );
    expect(current).toBe('Chat');
    await expect($('main#main textarea[aria-describedby="composer-hint"]')).toBeDisplayed();
  });
});
