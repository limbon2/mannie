// The desktop shell (M0): it opens into the working view, works from the
// keyboard and passes the WCAG 2.1 AA checks in both themes (NFR-UX-01,
// NFR-UX-03).
import { setTimeout as delay } from 'node:timers/promises';

import { Key } from 'webdriverio';

import { chooseConversationAction } from '../support/chat.ts';
import { chooseTrayItem, processId, seedConversation } from '../support/harness.ts';
import {
  axeViolations,
  focusedElement,
  focusIsVisible,
  isDark,
  leaveSettings,
  openSettings,
  runCommand,
  switchToPseudoLocale,
  uncataloguedText,
  waitForAnimations,
  waitForApp,
  waitForShownText,
} from '../support/page.ts';

const PRIMARY_COLORS = ['default', 'blue', 'teal', 'green', 'amber', 'violet', 'rose'];
const SURFACE_COLORS = ['neutral', 'slate', 'stone', 'olive'];

interface ContrastSample {
  readonly background: readonly string[];
  readonly foreground: string;
  readonly minimum: number;
  readonly name: string;
  readonly overlay?: string;
}

/** The browser resolves and composites the colors in the same sRGB space used to read luminance. */
async function lowContrast(samples: ContrastSample[]): Promise<string[]> {
  return browser.execute((pairs) => {
    const context = document.createElement('canvas').getContext('2d', { willReadFrequently: true });
    if (context === null) {
      throw new Error('Canvas is unavailable for reading computed theme colors');
    }

    function paint(canvas: CanvasRenderingContext2D, color: string): void {
      const sentinel = '#010203';
      canvas.fillStyle = sentinel;
      canvas.fillStyle = color;
      if (canvas.fillStyle === sentinel) {
        throw new Error(`Cannot read the theme color "${color}"`);
      }
      canvas.fillRect(0, 0, 1, 1);
    }

    function luminance(canvas: CanvasRenderingContext2D): number {
      const [red = 0, green = 0, blue = 0] = canvas.getImageData(0, 0, 1, 1).data;
      const linear = (channel: number): number => {
        const value = channel / 255;
        return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
      };
      return 0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue);
    }

    const failures: string[] = [];
    for (const { background, foreground, minimum, name, overlay } of pairs) {
      context.clearRect(0, 0, 1, 1);
      for (const layer of background) {
        paint(context, layer);
      }
      if (overlay !== undefined) {
        context.globalAlpha = 0.8;
        paint(context, overlay);
        context.globalAlpha = 1;
      }
      const fill = luminance(context);
      paint(context, foreground);
      const ink = luminance(context);
      const ratio = (Math.max(ink, fill) + 0.05) / (Math.min(ink, fill) + 0.05);
      if (ratio < minimum) {
        failures.push(`${name}: ${ratio.toFixed(2)}:1, expected ${String(minimum)}:1`);
      }
    }
    return failures;
  }, samples);
}

function isRunning(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

async function themeColors(): Promise<{ primary: string; surface: string }> {
  return browser.execute(() => {
    const style = getComputedStyle(document.documentElement);
    return {
      primary: style.getPropertyValue('--primary'),
      surface: style.getPropertyValue('--background'),
    };
  });
}

async function controlColors(selector: string): Promise<{
  background: string;
  border: string;
  foreground: string;
  shadow: string;
}> {
  return browser.execute((query) => {
    const control = document.querySelector(query);
    if (control === null) {
      throw new Error(`Cannot read the colors for ${query}`);
    }
    const style = getComputedStyle(control);
    return {
      background: style.backgroundColor,
      border: style.borderTopColor,
      foreground: style.color,
      shadow: style.boxShadow,
    };
  }, selector);
}

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
      { current: null, text: 'Agents' },
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

  it('reaches the skip link first, with a visible focus indicator', async () => {
    await browser.execute(() => {
      (document.activeElement as HTMLElement | null)?.blur();
    });
    await browser.keys(Key.Tab);
    expect(await focusedElement()).toEqual({ label: 'Skip to the main content', tag: 'a' });
    expect(await focusIsVisible()).toBe(true);
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
      'Skills',
      'Tools',
      'Child agents',
      'Window',
      'Plugins',
      'Updates',
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

  describe('interface colors', () => {
    beforeEach(async () => {
      await openSettings('shell.interface');
      await $('#interface-theme-light').click();
      await $('#interface-primary-default').click();
      await $('#interface-surface-neutral').click();
    });

    it('changes the two colors independently with the keyboard and pointer', async () => {
      const defaults = await themeColors();
      const blue = $('#interface-primary-blue');
      const slate = $('#interface-surface-slate');

      await $('#interface-theme-light').click();
      await browser.keys(Key.Tab);
      expect(await focusedElement()).toEqual({ label: 'Default', tag: 'button' });
      expect(await focusIsVisible()).toBe(true);
      await browser.keys(Key.ArrowRight);
      await expect(blue).toHaveAttribute('aria-checked', 'true');
      await waitForShownText('[data-slot="tooltip-content"]', 'Blue');
      const primary = await themeColors();
      expect(primary.primary).not.toBe(defaults.primary);
      expect(primary.surface).toBe(defaults.surface);

      await browser.keys(Key.Tab);
      expect(await focusedElement()).toEqual({ label: 'Neutral', tag: 'button' });
      await slate.moveTo();
      await waitForShownText('[data-slot="tooltip-content"]', 'Slate');
      await slate.click();
      await expect(slate).toHaveAttribute('aria-checked', 'true');
      const both = await themeColors();
      expect(both.primary).toBe(primary.primary);
      expect(both.surface).not.toBe(defaults.surface);
      expect(await axeViolations()).toEqual([]);

      await $('#interface-theme-dark').click();
      expect(await isDark()).toBe(true);
      await expect(blue).toHaveAttribute('aria-checked', 'true');
      await expect(slate).toHaveAttribute('aria-checked', 'true');
      const dark = await themeColors();
      expect(dark.primary).not.toBe(both.primary);
      expect(dark.surface).not.toBe(both.surface);
      expect(await axeViolations()).toEqual([]);

      await $('#interface-theme-system').click();
      await expect(blue).toHaveAttribute('aria-checked', 'true');
      await expect(slate).toHaveAttribute('aria-checked', 'true');
      await $('#interface-theme-light').click();
      expect(await themeColors()).toEqual(both);

      await $('#interface-primary-default').click();
      expect(await themeColors()).toEqual({ primary: defaults.primary, surface: both.surface });
      await $('#interface-surface-neutral').click();
      expect(await themeColors()).toEqual(defaults);
    });

    it('colors shared keyboard focus with Primary in both modes', async () => {
      async function focusLanguage(mode: 'dark' | 'light'): Promise<void> {
        await $(`#interface-theme-${mode}`).click();
        await browser.keys([Key.Shift, Key.Tab]);
        await expect($('#interface-language')).toBeFocused();
        await waitForAnimations();
      }

      for (const mode of ['light', 'dark'] as const) {
        await $('#interface-primary-default').click();
        await $('#interface-surface-slate').click();
        await focusLanguage(mode);
        const neutral = await controlColors('#interface-language:focus-visible');

        await $('#interface-primary-rose').click();
        await focusLanguage(mode);
        const accent = await controlColors('#interface-language:focus-visible');
        expect(accent.shadow).not.toBe('none');
        expect(accent.shadow).not.toBe(neutral.shadow);

        for (const surface of SURFACE_COLORS) {
          await $(`#interface-surface-${surface}`).click();
          await focusLanguage(mode);
          const { surface: background } = await themeColors();
          const { border, shadow } = await controlColors('#interface-language:focus-visible');
          expect(shadow).toMatch(/\b0px 0px 0px 3px\b/u);
          expect(
            await lowContrast([
              { background: [background], foreground: border, minimum: 3, name: `${mode}/${surface} focus border` },
            ]),
          ).toEqual([]);
        }
      }
    });

    it('colors tooltips and active choices with both palettes', async () => {
      const tooltip = '[data-slot="tooltip-content"]';
      const currentSection = '[data-shell-sidebar="content"] [data-sidebar="menu-button"][aria-current="page"]';
      const selectedOption = '[data-slot="select-item"][aria-selected="true"]';
      const currentCommand = '[data-slot="command-item"][aria-selected="true"]';

      async function sampleColors() {
        const nav = await controlColors(currentSection);
        await $('[data-field-help][aria-label="Help: Language"]').moveTo();
        await waitForShownText(tooltip, /Changing the language/u);
        await waitForAnimations();
        const tip = await controlColors(tooltip);
        const arrow = await controlColors(`${tooltip} > div[data-side]`);
        expect(arrow.background).toBe(tip.background);

        await $('#interface-language').click();
        await $(selectedOption).waitForDisplayed();
        await waitForAnimations();
        const choice = await controlColors(selectedOption);
        await browser.keys(Key.Escape);
        await $(selectedOption).waitForDisplayed({ reverse: true });

        await browser.keys([Key.Ctrl, 'k']);
        await $('[data-slot="command-input"]').waitForDisplayed();
        await browser.keys('Show Chat'.split(''));
        await waitForShownText(currentCommand, 'Show Chat');
        await waitForAnimations();
        const command = await controlColors(currentCommand);
        await browser.keys(Key.Escape);
        await $('[data-slot="command-input"]').waitForDisplayed({ reverse: true });

        expect(
          await lowContrast([
            { background: [tip.background], foreground: tip.foreground, minimum: 4.5, name: 'tooltip' },
          ]),
        ).toEqual([]);
        return [nav.background, tip.background, choice.background, command.background];
      }

      const defaults = await sampleColors();
      await $('#interface-primary-rose').click();
      const primary = await sampleColors();
      await $('#interface-surface-slate').click();
      const both = await sampleColors();
      for (const [index, color] of both.entries()) {
        expect(primary[index]).not.toBe(defaults[index]);
        expect(color).not.toBe(primary[index]);
      }
    });

    it('restores both choices after reloading and restarting the app', async () => {
      await $('#interface-primary-teal').click();
      await $('#interface-surface-stone').click();
      await $('#interface-theme-dark').click();
      const colors = await themeColors();

      await browser.refresh();
      await waitForApp();
      expect(await themeColors()).toEqual(colors);
      await openSettings('shell.interface');
      await expect($('#interface-primary-teal')).toHaveAttribute('aria-checked', 'true');
      await expect($('#interface-surface-stone')).toHaveAttribute('aria-checked', 'true');

      // A normal quit lets the webview flush its saved preferences before
      // WebDriver opens the next session.
      const pid = await processId();
      try {
        await chooseTrayItem('quit');
      } catch (error) {
        // WebKit can end the session before the Quit response arrives.
        const sessionEnded =
          error instanceof Error &&
          /unknown error when running "execute\/sync"|invalid session id/u.test(error.message);
        if (!sessionEnded) {
          throw error;
        }
      }
      const deadline = Date.now() + 15_000;
      while (isRunning(pid)) {
        if (Date.now() > deadline) {
          throw new Error('Mannie did not quit');
        }
        await delay(250);
      }
      await browser.reloadSession();
      await waitForApp();
      expect(await themeColors()).toEqual(colors);
      await openSettings('shell.interface');
      await expect($('#interface-primary-teal')).toHaveAttribute('aria-checked', 'true');
      await expect($('#interface-surface-stone')).toHaveAttribute('aria-checked', 'true');
    });

    it('falls back only for the choice with an invalid saved value', async () => {
      await browser.execute(() => {
        localStorage.setItem('mannie.theme.primary', 'blue palette-primary-rose');
        localStorage.setItem('mannie.theme.surface', 'olive');
      });
      await browser.refresh();
      await waitForApp();
      await openSettings('shell.interface');
      await expect($('#interface-primary-default')).toHaveAttribute('aria-checked', 'true');
      await expect($('#interface-surface-olive')).toHaveAttribute('aria-checked', 'true');
      await expect($('html')).not.toHaveElementClass('palette-primary-rose');

      await browser.execute(() => {
        localStorage.setItem('mannie.theme.primary', 'blue');
        localStorage.setItem('mannie.theme.surface', 'unknown');
      });
      await browser.refresh();
      await waitForApp();
      await openSettings('shell.interface');
      await expect($('#interface-primary-blue')).toHaveAttribute('aria-checked', 'true');
      await expect($('#interface-surface-neutral')).toHaveAttribute('aria-checked', 'true');
    });

    it('applies a choice to the current window when storage is unavailable', async () => {
      const defaults = await themeColors();
      await browser.execute(() => {
        const original = Reflect.getOwnPropertyDescriptor(window, 'localStorage');
        Object.defineProperty(window, 'localStorage', {
          configurable: true,
          get() {
            throw new DOMException('Storage blocked by the test', 'SecurityError');
          },
        });
        try {
          document.querySelector<HTMLButtonElement>('#interface-primary-violet')?.click();
        } finally {
          if (original === undefined) {
            Reflect.deleteProperty(window, 'localStorage');
          } else {
            Object.defineProperty(window, 'localStorage', original);
          }
        }
      });
      await expect($('#interface-primary-violet')).toHaveAttribute('aria-checked', 'true');
      expect((await themeColors()).primary).not.toBe(defaults.primary);

      await browser.refresh();
      await waitForApp();
      expect(await themeColors()).toEqual(defaults);
      await openSettings('shell.interface');
      await expect($('#interface-primary-default')).toHaveAttribute('aria-checked', 'true');
    });

    it('keeps the hovered Delete label readable on every Surface in both modes', async () => {
      const dialog = '[data-slot="alert-dialog-content"]';
      const footer = '[data-slot="alert-dialog-footer"]';
      const deleteButton = `${footer} button[data-slot="button"]`;
      const conversation = await seedConversation(1);
      const url = new URL(await browser.getUrl());
      url.pathname = '/';
      url.search = new URLSearchParams({ conversation, view: 'chat' }).toString();

      const samples: ContrastSample[] = [];
      for (const mode of ['light', 'dark']) {
        for (const surface of SURFACE_COLORS) {
          await openSettings('shell.interface');
          await $(`#interface-theme-${mode}`).click();
          await $(`#interface-surface-${surface}`).click();
          await browser.url(url.toString());
          await waitForApp();
          await chooseConversationAction('Delete');
          await waitForShownText(deleteButton, 'Delete');
          await $(deleteButton).moveTo();
          await waitForAnimations();

          const hovered = `${deleteButton}:hover`;
          const { background, foreground } = await controlColors(hovered);
          const dialogFill = await controlColors(dialog);
          const footerFill = await controlColors(footer);
          samples.push({
            background: [dialogFill.background, footerFill.background, background],
            foreground,
            minimum: 4.5,
            name: `${mode}/${surface} hovered Delete`,
          });

          await browser.keys(Key.Escape);
          await $(dialog).waitForDisplayed({ reverse: true });
        }
      }
      expect(await lowContrast(samples)).toEqual([]);
    });

    it('keeps text, hovered primary buttons, tooltips and swatch focus visible across the palette', async () => {
      await $('#interface-theme-light').click();
      await browser.keys(Key.Tab);
      expect(await focusedElement()).toEqual({ label: 'Default', tag: 'button' });
      await waitForShownText('[data-slot="tooltip-content"]', 'Default');
      await waitForAnimations();

      const { failures, samples } = await browser.execute(
        (primaries: string[], surfaces: string[]) => {
          const root = document.documentElement;
          const originalClass = root.className;
          const failures: string[] = [];
          const samples: ContrastSample[] = [];

          const layers = ['--background', '--card', '--popover', '--muted', '--secondary', '--accent', '--sidebar'];
          const swatch = document.getElementById('interface-primary-default');
          const tooltip = document.querySelector('[data-slot="tooltip-content"]');
          const activeNav = document.querySelector('[data-shell-sidebar="content"] [aria-current="page"]');
          const hasControls = swatch !== null && tooltip !== null && activeNav !== null;
          if (!hasControls) {
            throw new Error('The primary swatch, tooltip or active Settings row is missing');
          }
          try {
            for (const mode of ['light', 'dark']) {
              for (const primary of primaries) {
                for (const surface of surfaces) {
                  const description = `${mode}/${primary}/${surface}`;
                  root.classList.toggle('dark', mode === 'dark');
                  root.classList.remove(
                    ...primaries.map((value) => `palette-primary-${value}`),
                    ...surfaces.map((value) => `palette-surface-${value}`),
                  );
                  if (primary !== 'default') {
                    root.classList.add(`palette-primary-${primary}`);
                  }
                  if (surface !== 'neutral') {
                    root.classList.add(`palette-surface-${surface}`);
                  }

                  const style = getComputedStyle(root);
                  const primaryColor = style.getPropertyValue('--primary');
                  const primaryText = style.getPropertyValue('--primary-foreground');
                  const muted = style.getPropertyValue('--muted-foreground');
                  samples.push({
                    background: [primaryColor],
                    foreground: primaryText,
                    minimum: 4.5,
                    name: `${description} primary button text`,
                  });
                  const tooltipStyle = getComputedStyle(tooltip);
                  const navStyle = getComputedStyle(activeNav);
                  samples.push(
                    {
                      background: [tooltipStyle.backgroundColor],
                      foreground: tooltipStyle.color,
                      minimum: 4.5,
                      name: `${description} tooltip text`,
                    },
                    {
                      background: [style.getPropertyValue('--sidebar'), navStyle.backgroundColor],
                      foreground: navStyle.color,
                      minimum: 4.5,
                      name: `${description} active navigation text`,
                    },
                  );
                  for (const layer of layers) {
                    const background = style.getPropertyValue(layer);
                    samples.push(
                      {
                        background: [background],
                        foreground: primaryColor,
                        minimum: 4.5,
                        name: `${description} primary text on ${layer}`,
                      },
                      {
                        background: [background],
                        foreground: muted,
                        minimum: 4.5,
                        name: `${description} muted text on ${layer}`,
                      },
                      {
                        background: [background],
                        foreground: primaryText,
                        minimum: 4.5,
                        name: `${description} hovered button text on ${layer}`,
                        overlay: primaryColor,
                      },
                    );
                  }
                  const focus = getComputedStyle(swatch);
                  if (focus.outlineStyle === 'none') {
                    failures.push(`${description} has no swatch focus outline`);
                  } else {
                    samples.push({
                      background: [style.getPropertyValue('--background')],
                      foreground: focus.outlineColor,
                      minimum: 3,
                      name: `${description} swatch focus outline`,
                    });
                  }
                }
              }
            }
          } finally {
            root.className = originalClass;
          }
          return { failures, samples };
        },
        PRIMARY_COLORS,
        SURFACE_COLORS,
      );
      expect([...failures, ...(await lowContrast(samples))]).toEqual([]);
    });

    it('fits a narrow window and uses the pseudo-locale for swatch names and tooltips', async () => {
      const { height, width } = await browser.getWindowSize();
      const originalLocale = await browser.execute(() => document.documentElement.lang);
      try {
        await browser.setWindowSize(800, height);
        await switchToPseudoLocale();
        await openSettings('shell.interface');
        await $('#interface-theme-light').click();
        await browser.keys(Key.Tab);
        await browser.keys(Key.ArrowRight);
        await waitForShownText('[data-slot="tooltip-content"]', /⟦.*⟧/u);
        expect(await uncataloguedText()).toEqual([]);

        const layout = await browser.execute(() => {
          const viewport = document.documentElement.clientWidth;
          const groups = Array.from(document.querySelectorAll('[role="radiogroup"]')).filter(
            (group) => group.querySelector('[id^="interface-primary-"], [id^="interface-surface-"]') !== null,
          );
          return groups.map((group) => {
            const bounds = group.getBoundingClientRect();
            const swatches = Array.from(group.querySelectorAll('[role="radio"]'));
            const staysInViewport = bounds.right <= viewport;
            const swatchesFit = swatches.every((swatch) => {
              const rect = swatch.getBoundingClientRect();
              const startsWithinGroup = rect.left >= bounds.left;
              const endsWithinGroup = rect.right <= bounds.right;
              return startsWithinGroup && endsWithinGroup;
            });
            return {
              count: swatches.length,
              fits: staysInViewport && swatchesFit,
            };
          });
        });
        expect(layout).toEqual([
          { count: PRIMARY_COLORS.length, fits: true },
          { count: SURFACE_COLORS.length, fits: true },
        ]);
        expect(await axeViolations()).toEqual([]);
      } finally {
        await browser.setWindowSize(width, height);
        if ((await browser.execute(() => document.documentElement.lang)) !== originalLocale) {
          await openSettings('shell.interface');
          await $('#interface-language').click();
          await $('[data-slot="select-item"][data-value="*"]').click();
          await browser.waitUntil(
            async () => (await browser.execute(() => document.documentElement.lang)) === originalLocale,
          );
        }
      }
    });
  });
});
