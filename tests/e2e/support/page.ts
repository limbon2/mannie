import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';

// Helpers that run in the app's webview.
import type { AxeResults } from 'axe-core';

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

/** Runs axe's WCAG 2.1 AA rules on the page and returns the violations. */
export async function axeViolations(): Promise<AxeViolation[]> {
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
export async function waitForShownText(selector: string, expected: RegExp | string): Promise<void> {
  const matches = (text: string): boolean => (typeof expected === 'string' ? text === expected : expected.test(text));
  let last = '';
  await browser.waitUntil(
    async () => {
      const element = await $(selector);
      const displayed = (await element.isExisting()) && (await element.isDisplayed());
      if (!displayed) {
        return false;
      }
      last = await browser.execute(
        (found: string) => document.querySelector(found)?.textContent.trim() ?? '',
        selector,
      );
      return matches(last);
    },
    { timeout: 10_000, timeoutMsg: `${selector} did not show ${String(expected)}; it showed ${JSON.stringify(last)}` },
  );
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
 * readers announce or that show as text.
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
    const problems: string[] = [];
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
      const text = node.textContent?.trim() ?? '';
      const needsMarks = hasLetters(text) && !isMarked(text);
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
