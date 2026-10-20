import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';

// Helpers that run in the app's webview.
import type { AxeResults } from 'axe-core';
import { Key } from 'webdriverio';

const axeSource = readFileSync(createRequire(import.meta.url).resolve('axe-core/axe.min.js'), 'utf8');

/** The WCAG 2.1 AA rules (NFR-UX-01). */
const WCAG_TAGS = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa'];

/** One accessibility problem axe found, with the elements it found it on. */
export interface AxeViolation {
  readonly help: string;
  readonly id: string;
  readonly impact: string | null;
  readonly targets: string[];
}

declare global {
  interface Window {
    axe?: { run(context: Document, options: object): Promise<AxeResults> };
  }
}

/** Waits until the shell, or the startup problem view, has rendered. */
export async function waitForApp(): Promise<void> {
  await browser.waitUntil(async () => browser.execute(() => document.querySelector('main') !== null), {
    timeout: 30_000,
    timeoutMsg: 'the app did not render its main area',
  });
}

/**
 * Waits until the page's finite animations, such as a dialog that fades in,
 * have finished. Animations that run forever, such as spinners, do not count.
 */
export async function waitForAnimations(): Promise<void> {
  await browser.waitUntil(
    async () =>
      browser.execute(() =>
        document.getAnimations().every((animation) => {
          const runsForever = animation.effect?.getComputedTiming().endTime === Infinity;
          const isRunning = animation.playState === 'running';
          return runsForever || !isRunning;
        }),
      ),
    { timeout: 5_000, timeoutMsg: 'the page kept animating' },
  );
}

/**
 * Runs axe's WCAG 2.1 AA rules on the page and returns the violations. It
 * waits for animations first: colors measured halfway through a fade are not
 * the colors the owner reads.
 */
export async function axeViolations(): Promise<AxeViolation[]> {
  await waitForAnimations();
  await browser.execute(axeSource);
  return browser.execute(async (tags: string[]) => {
    const axe = window.axe;
    if (axe === undefined) {
      throw new Error('axe did not load');
    }
    const results = await axe.run(document, { runOnly: { type: 'tag', values: tags } });
    return results.violations.map((violation) => ({
      help: violation.help,
      id: violation.id,
      impact: violation.impact ?? null,
      targets: violation.nodes.map((node) => `${node.target.join(' ')}: ${node.failureSummary ?? node.html}`),
    }));
  }, WCAG_TAGS);
}

/**
 * Waits until the element is displayed and its text is `expected`, or
 * contains it when `expected` is a regular expression. It reads the DOM text:
 * WebKitWebDriver reports no visible text for text in a box that truncates
 * it, such as the header's heading and the value of a select.
 */
export async function waitForShownText(
  target: ReturnType<typeof $> | string | WebdriverIO.Element,
  expected: RegExp | string | { asymmetricMatch(value: unknown): boolean },
): Promise<void> {
  const matches = (text: string): boolean => {
    if (typeof expected === 'string') {
      return text === expected;
    }
    if (expected instanceof RegExp) {
      return expected.test(text);
    }
    return expected.asymmetricMatch(text);
  };
  let last = '';
  const element = typeof target === 'string' ? await $(target) : await target;
  const label = typeof element.selector === 'string' ? element.selector : 'the element';
  let expectedText = 'matching text';
  if (typeof expected === 'string') {
    expectedText = JSON.stringify(expected);
  }
  if (expected instanceof RegExp) {
    expectedText = expected.toString();
  }
  await browser.waitUntil(
    async () => {
      const displayed = (await element.isExisting()) && (await element.isDisplayed());
      if (!displayed) {
        return false;
      }
      const content: unknown = await element.getProperty('textContent');
      last = typeof content === 'string' ? content.trim() : '';
      return matches(last);
    },
    {
      timeout: 10_000,
      timeoutMsg: `${label} did not show ${expectedText}; it showed ${JSON.stringify(last)}`,
    },
  );
}

/**
 * Opens the settings with Ctrl+, and then the page of the section `id`, such
 * as `chat.providers`, from the settings sidebar. The sidebar's links are
 * found by their address, never their text, so this works in the pseudo-locale.
 */
export async function openSettings(id: string): Promise<void> {
  await browser.keys([Key.Ctrl, ',']);
  const entry = await $(`[data-slot="sidebar"] a[href$="?section=${id}"]`);
  await entry.waitForClickable();
  await entry.click();
  await browser.waitUntil(async () => (await entry.getAttribute('aria-current')) === 'page', {
    timeout: 10_000,
    timeoutMsg: `the settings page of ${id} did not open`,
  });
}

/** Leaves the settings through the link back to the main view, when they are open. */
export async function leaveSettings(): Promise<void> {
  const onSettings = await browser.execute(() => location.pathname.endsWith('/settings'));
  if (!onSettings) {
    return;
  }
  // The settings sidebar's header holds only that link.
  await $('[data-slot="sidebar-header"] a').click();
  await browser.waitUntil(async () => browser.execute(() => !location.pathname.endsWith('/settings')), {
    timeout: 10_000,
    timeoutMsg: 'the settings did not close',
  });
}

/**
 * Opens what a plugin may use on the Plugins page, from the keyboard. The
 * plugin is found by its ID, so this works in the pseudo-locale.
 */
export async function openPluginUses(id: string): Promise<void> {
  await browser.execute((selector: string) => {
    document.querySelector<HTMLElement>(selector)?.focus();
  }, `[data-plugin="${id}"] [data-plugin-uses-trigger]`);
  await browser.keys(Key.Enter);
  await $(`[data-plugin="${id}"] [data-plugin-uses]`).waitForDisplayed();
}

/** The pseudo-locale of the validation builds (scripts/pseudo-locale.mjs). */
export const PSEUDO_LOCALE = 'en-XA';

/** Shows the interface in the pseudo-locale, from the interface settings, and waits until the window does. */
export async function switchToPseudoLocale(): Promise<void> {
  await openSettings('shell.interface');
  await $('#interface-language').waitForDisplayed();
  await $('#interface-language').click();
  await $(`[data-slot="select-item"][data-value="${PSEUDO_LOCALE}"]`).click();
  await browser.waitUntil(async () => (await browser.execute(() => document.documentElement.lang)) === PSEUDO_LOCALE, {
    timeout: 15_000,
    timeoutMsg: 'the window did not show the pseudo-locale',
  });
  await waitForApp();
}

/**
 * Turns a plugin on or off with its switch on the Plugins page, and waits
 * until the core reports the state that the switch asked for. The switch is
 * found by the plugin's ID, so this works in the pseudo-locale. A plugin with
 * work in progress asks before it turns off, which this does not answer.
 */
export async function setPluginEnabled(id: string, enabled: boolean): Promise<void> {
  await openSettings('plugins.settings');
  const plugin = await $(`[data-plugin="${id}"]`);
  await plugin.waitForDisplayed();
  const wanted = enabled ? 'on' : 'off';
  if ((await plugin.getAttribute('data-plugin-state')) === wanted) {
    return;
  }
  await plugin.$('[data-plugin-switch]').click();
  await $(`[data-plugin="${id}"][data-plugin-state="${wanted}"]`).waitForExist({
    timeout: 30_000,
    timeoutMsg: `the plugin ${id} did not turn ${wanted}`,
  });
}

/** Whether the window shows the dark theme. */
export async function isDark(): Promise<boolean> {
  return browser.execute(() => document.documentElement.classList.contains('dark'));
}

/**
 * Text a person can see or hear that did not come from the message catalog.
 * In the pseudo-locale every catalog message is wrapped in ⟦ and ⟧
 * (scripts/pseudo-locale.mjs), so any other text with letters bypassed the
 * catalog (ADR-0019). It checks text nodes and the attributes that screen
 * readers announce or that show as text. Text inside an element marked
 * `data-content` is the owner's or a model's, such as messages, replies and
 * names, and needs no catalog; a catalog message that names such content is
 * still marked as a whole.
 */
export async function uncataloguedText(): Promise<string[]> {
  return browser.execute(() => {
    const hasLetters = (text: string): boolean => /\p{L}/u.test(text);
    const isMarked = (text: string): boolean => text.includes('⟦') && text.includes('⟧');
    const isShown = (element: Element | null): boolean => {
      if (element === null) {
        return false;
      }
      const hidden = element.closest('[aria-hidden="true"], script, style, template, noscript');
      if (hidden !== null) {
        return false;
      }
      const style = getComputedStyle(element);
      const isDisplayed = style.display !== 'none';
      const isVisible = style.visibility !== 'hidden';
      return isDisplayed && isVisible;
    };
    const isContent = (element: Element | null): boolean => {
      if (element === null) {
        return false;
      }
      // SvelteKit's route announcer repeats the page's title, which the
      // header shows and this check covers there.
      return element.closest('[data-content], #svelte-announcer') !== null;
    };
    const problems: string[] = [];
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
      const text = node.textContent?.trim() ?? '';
      const needsMarks = hasLetters(text) && !isMarked(text) && !isContent(node.parentElement);
      if (needsMarks && isShown(node.parentElement)) {
        problems.push(text);
      }
    }
    const attributes = [
      'aria-label',
      'aria-description',
      'aria-placeholder',
      'aria-valuetext',
      'alt',
      'placeholder',
      'title',
    ];
    for (const element of document.querySelectorAll(attributes.map((name) => `[${name}]`).join(','))) {
      for (const name of attributes) {
        const value = element.getAttribute(name)?.trim() ?? '';
        const needsMarks = hasLetters(value) && !isMarked(value);
        if (needsMarks && isShown(element)) {
          problems.push(`${name}="${value}"`);
        }
      }
    }
    return problems;
  });
}

/** Runs a command of the command palette by typing its label, as the owner would. */
export async function runCommand(label: string): Promise<void> {
  await browser.keys([Key.Ctrl, 'k']);
  const input = await $('[data-slot="command-input"]');
  await input.waitForDisplayed();
  await browser.keys(label.split(''));
  await browser.keys(Key.Enter);
  await $('[data-slot="command-input"]').waitForDisplayed({ reverse: true });
}

/** The element that has the keyboard focus, described for assertions. */
export async function focusedElement(): Promise<{ label: string; tag: string }> {
  return browser.execute(() => {
    const element = document.activeElement;
    if (element === null) {
      return { label: '', tag: '' };
    }
    const label = element.getAttribute('aria-label') ?? element.textContent.trim();
    return { label, tag: element.tagName.toLowerCase() };
  });
}

/** Whether the focused element shows a focus indicator: an outline or a ring drawn with a box shadow. */
export async function focusIsVisible(): Promise<boolean> {
  return browser.execute(() => {
    const element = document.activeElement;
    const nothingFocused = element === null || element === document.body;
    if (nothingFocused) {
      return false;
    }
    const style = getComputedStyle(element);
    const hasOutline = style.outlineStyle !== 'none' && Number.parseFloat(style.outlineWidth) > 0;
    const hasRing = style.boxShadow !== 'none';
    return hasOutline || hasRing;
  });
}
