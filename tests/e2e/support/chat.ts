// The chat views as the end-to-end tests use them (docs/features/chat/
// conversation-view.md). The selectors use the structure and data attributes
// of the views, never their English text, so the helpers also work in the
// pseudo-locale.
import { Key } from 'webdriverio';

import { createProviderProfile, type ProfileInput } from './harness.ts';
import { leaveSettings, waitForAnimations } from './page.ts';

/** The message box. */
export const COMPOSER = 'textarea[aria-describedby="composer-hint"]';

/** The credential of the test profiles; the test provider accepts any. */
export const TEST_CREDENTIAL = 'e2e-credential';

/** A profile for the test provider. */
export interface ProfileOptions {
  readonly contextWindow?: number;
  readonly credential?: string | null;
  readonly interface?: ProfileInput['draft']['interface'];
  readonly model: string;
  readonly name?: string;
  readonly outputLimit?: number;
}

/** A part as the transcript shows it. */
export interface ShownPart {
  readonly kind: string;
  readonly state: string;
  readonly text: string;
}

/**
 * Clicks the displayed element of `selector` whose text is `text`, once
 * menus and dialogs have stopped moving, so that the click lands on it.
 */
export async function clickWithText(selector: string, text: string): Promise<void> {
  await waitForAnimations();
  await browser.waitUntil(
    async () => {
      for (const element of await $$(selector)) {
        const shown = (await element.isDisplayed()) && (await element.getText()).trim() === text;
        if (shown) {
          await element.click();
          return true;
        }
      }
      return false;
    },
    { timeout: 10_000, timeoutMsg: `no ${selector} shows "${text}"` },
  );
}

/** Opens the open conversation's menu in the toolbar and chooses an item. */
export async function chooseConversationAction(label: string): Promise<void> {
  await waitForPointerInput();
  await $('main button[aria-label^="More actions for"]').click();
  await clickWithText('[data-slot="dropdown-menu-item"]', label);
}

/** Makes the profile named `name` the open conversation's profile. */
export async function useProfile(name: string): Promise<void> {
  await waitForPointerInput();
  await $('main button[aria-label^="Provider profile"]').click();
  await clickWithText('[data-slot="dropdown-menu-radio-item"]', name);
  await expect($('main button[aria-label^="Provider profile"]')).toHaveText(name);
}

/** Adds a profile for the test provider at `baseUrl`. The first profile becomes the default. */
export async function addProfile(baseUrl: string, options: ProfileOptions): Promise<string> {
  return createProviderProfile({
    credential: options.credential === undefined ? TEST_CREDENTIAL : options.credential,
    draft: {
      baseUrl,
      contextWindow: options.contextWindow ?? 32_768,
      interface: options.interface ?? 'chat_completions',
      model: options.model,
      name: options.name ?? options.model,
      outputLimit: options.outputLimit ?? 4_096,
      reasoningEffort: 'server_default',
    },
  });
}

/** Waits until the message box takes a message. */
export async function waitForComposer(): Promise<void> {
  await browser.waitUntil(
    async () =>
      browser.execute((selector: string) => {
        const box = document.querySelector<HTMLTextAreaElement>(selector);
        if (box === null) {
          return false;
        }
        return !box.disabled;
      }, COMPOSER),
    { timeout: 15_000, timeoutMsg: 'the message box did not take messages' },
  );
}

/** Starts a new conversation with the main sidebar's button, leaving the settings first when they are open. */
export async function startNewConversation(): Promise<void> {
  await leaveSettings();
  await $('[data-slot="sidebar-group-action"]').click();
  await browser.waitUntil(async () => browser.execute(() => document.querySelectorAll('main article').length === 0), {
    timeout: 10_000,
    timeoutMsg: 'the new conversation did not open',
  });
  await waitForComposer();
}

/** Clicks Retry below the latest reply's status, a button that shows only its icon. */
export async function retryLatestReply(): Promise<void> {
  await waitForAnimations();
  await $('main button[aria-label="Retry"]').click();
}

/**
 * Waits until the page takes pointer input. While a menu or a dialog is open,
 * and for a moment after it closes, bits-ui turns pointer events off for the
 * rest of the page.
 */
export async function waitForPointerInput(): Promise<void> {
  await browser.waitUntil(async () => browser.execute(() => getComputedStyle(document.body).pointerEvents !== 'none'), {
    timeout: 5_000,
    timeoutMsg: 'the page did not take pointer input again',
  });
}

/** Types a message into the message box and sends it with Enter. */
export async function sendMessage(text: string): Promise<void> {
  await waitForComposer();
  await waitForPointerInput();
  const box = await $(COMPOSER);
  await box.click();
  await box.setValue(text);
  await browser.keys(Key.Enter);
}

/** How many turns the transcript shows. */
export async function turnCount(): Promise<number> {
  return browser.execute(() => document.querySelectorAll('main article').length);
}

/** The parts of the latest turn, in order. */
export async function latestTurnParts(): Promise<ShownPart[]> {
  return browser.execute(() => {
    const turns = document.querySelectorAll('main article');
    const latest = turns.item(turns.length - 1) as Element | null;
    if (latest === null) {
      return [];
    }
    return [...latest.querySelectorAll<HTMLElement>('[data-part-kind]')].map((element) => ({
      kind: element.dataset.partKind ?? '',
      state: element.dataset.partState ?? '',
      text: element.textContent.trim(),
    }));
  });
}

/**
 * Waits until the latest turn has a reply of `kind` in one of `states`, and
 * returns it.
 */
export async function waitForLatestPart(kind: string, states: readonly string[], timeout = 30_000): Promise<ShownPart> {
  let found: ShownPart | undefined;
  let seen: ShownPart[] = [];
  await browser.waitUntil(
    async () => {
      seen = await latestTurnParts();
      found = seen.find((part) => {
        const isKind = part.kind === kind;
        const isInState = states.includes(part.state);
        return isKind && isInState;
      });
      return found !== undefined;
    },
    {
      timeout,
      timeoutMsg: `the latest turn has no ${kind} part in ${states.join(' or ')}; it has ${JSON.stringify(seen)}`,
    },
  );
  if (found === undefined) {
    throw new Error('unreachable: waitUntil resolved without a part');
  }
  return found;
}

/** Waits until the latest reply is complete, and returns its text. */
export async function waitForCompleteReply(timeout = 30_000): Promise<string> {
  const part = await waitForLatestPart('text', ['complete'], timeout);
  return part.text;
}

/** The conversations in the sidebar, by their links' titles. */
export async function listedConversations(): Promise<string[]> {
  return browser.execute(() =>
    [...document.querySelectorAll('[data-slot="sidebar"] a[href*="conversation="]')].map((link) =>
      link.textContent.trim(),
    ),
  );
}
