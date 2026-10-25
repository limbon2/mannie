// The chat views as the end-to-end tests use them (docs/features/chat/
// conversation-view.md). The selectors use the structure and data attributes
// of the views, never their English text, so the helpers also work in the
// pseudo-locale.
import { Key } from 'webdriverio';

import { createProviderProfile, type ProfileInput, setFolderChoice } from './harness.ts';
import { leaveSettings, waitForAnimations, waitForShownText } from './page.ts';

/** The message box. */
export const COMPOSER = 'textarea[aria-describedby="composer-hint"]';

/**
 * The turns of the main view's transcript. An open side panel sits in the
 * main area and can show a transcript of its own (ADR-0025), which these
 * helpers leave out.
 */
export const MAIN_TURNS = 'main article:not([data-side-panels] article)';

/** The latest turn of the main view's transcript. */
export const LATEST_MAIN_TURN = 'main article:last-of-type:not([data-side-panels] article)';

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
  /**
   * Whether the model gets tools. Off unless a test asks, so that the chat
   * tests send what chat sent before system tools, as a profile without tools
   * does (docs/features/system-tools/README.md, fit).
   */
  readonly useTools?: boolean;
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
        const content: unknown = await element.getProperty('textContent');
        const shown = (await element.isDisplayed()) && typeof content === 'string' && content.trim() === text;
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

/** The open conversation's row in the sidebar. */
export const OPEN_CONVERSATION_ROW = '[data-conversation]:has(a[aria-current="page"])';

/** Opens the open conversation's menu in the sidebar, Rename and Delete, and waits until it shows. */
export async function openConversationRowMenu(): Promise<void> {
  await waitForPointerInput();
  const row = $(OPEN_CONVERSATION_ROW);
  // The menu's button shows while the pointer is on the row.
  await row.moveTo();
  await row.$('[data-conversation-menu]').click();
  await $('[data-slot="dropdown-menu-content"]').waitForDisplayed();
}

/** Opens the open conversation's menu in the sidebar and chooses an item. */
export async function chooseConversationAction(label: string): Promise<void> {
  await openConversationRowMenu();
  await clickWithText('[data-slot="dropdown-menu-item"]', label);
}

/**
 * The button of the main view's conversation menu at the end of its toolbar,
 * which counts running work (ADR-0042). A conversation in a side panel has a
 * menu of its own.
 */
export const CONVERSATION_ACTIONS = '#shell-main-pane [data-conversation-actions]';

/** The open conversation's menu. */
export const CONVERSATION_MENU = '[data-slot="dropdown-menu-content"]';

/** The submenu of the conversation's menu that is open. */
export const CONVERSATION_SUBMENU = '[data-slot="dropdown-menu-sub-content"]';

/** Opens the open conversation's menu and waits until it shows. */
export async function openConversationActions(): Promise<void> {
  await waitForPointerInput();
  await $(CONVERSATION_ACTIONS).click();
  await $(CONVERSATION_MENU).waitForDisplayed();
}

/** Opens the open conversation's menu, then its submenu whose trigger matches `trigger`, and waits until it shows. */
export async function openConversationSubmenu(trigger: string): Promise<void> {
  await openConversationActions();
  await $(`${CONVERSATION_MENU} ${trigger}`).click();
  await $(CONVERSATION_SUBMENU).waitForDisplayed();
}

/** Opens the open conversation's context meter and chooses an item of its menu, such as "Compact now". */
export async function chooseContextAction(label: string): Promise<void> {
  await waitForPointerInput();
  await $('#shell-main-pane [data-context-meter]').click();
  await clickWithText('[data-slot="dropdown-menu-item"]', label);
}

/** Makes the profile named `name` the open conversation's profile. */
export async function useProfile(name: string): Promise<void> {
  await waitForPointerInput();
  await $('main button[aria-label^="Provider profile"]').click();
  await clickWithText('[data-slot="dropdown-menu-radio-item"]', name);
  await waitForShownText($('main button[aria-label^="Provider profile"]'), name);
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
      useTools: options.useTools ?? false,
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

/** Waits until the pane of a new conversation shows, with the message box ready. */
async function waitForNewConversation(): Promise<void> {
  await browser.waitUntil(
    async () => browser.execute((turns: string) => document.querySelectorAll(turns).length === 0, MAIN_TURNS),
    { timeout: 10_000, timeoutMsg: 'the new conversation did not open' },
  );
  await waitForComposer();
}

/**
 * Starts a new general conversation with the "+" of the sidebar's
 * "Conversations" group, leaving the settings first when they are open.
 */
export async function startNewConversation(): Promise<void> {
  await leaveSettings();
  await $('[data-new-conversation]').click();
  await waitForNewConversation();
}

/** The project in the address of the new conversation's pane, if it names one. */
async function shownProject(): Promise<string | null> {
  return browser.execute(() => new URLSearchParams(location.search).get('project'));
}

/**
 * Adds a folder as a project with the "+" of the sidebar's "Projects" group,
 * answering the folder dialog with `folder`, and returns the project's ID once
 * its new conversation opened.
 */
export async function addProject(folder: string): Promise<string> {
  await leaveSettings();
  await setFolderChoice(folder);
  await $('[data-add-project]').click();
  await browser.waitUntil(async () => (await shownProject()) !== null, {
    timeout: 10_000,
    timeoutMsg: 'the project did not open',
  });
  await waitForNewConversation();
  const project = await shownProject();
  if (project === null) {
    throw new Error('the project did not open');
  }
  return project;
}

/** The row of a project in the sidebar. */
export function projectRow(projectId: string): ReturnType<typeof $> {
  return $(`[data-project="${projectId}"]`);
}

/** The button that collapses and expands a project. */
export function projectToggle(projectId: string): ReturnType<typeof $> {
  return $(`[data-project="${projectId}"] [data-project-toggle]`);
}

/**
 * Starts a new conversation in a project with the button that its row shows
 * on hover, leaving the settings first when they are open.
 */
export async function startConversationIn(projectId: string): Promise<void> {
  await leaveSettings();
  await waitForPointerInput();
  await projectToggle(projectId).moveTo();
  await $(`[data-new-conversation-in="${projectId}"]`).click();
  await browser.waitUntil(async () => (await shownProject()) === projectId, {
    timeout: 10_000,
    timeoutMsg: 'the new conversation of the project did not open',
  });
  await waitForNewConversation();
}

/** Chooses how long the latest turn's waiting request is allowed, from its "Allow for…" menu. */
export async function allowFor(label: string): Promise<void> {
  await waitForPointerInput();
  const trigger = $(`${LATEST_MAIN_TURN} [data-allow-for]`);
  await trigger.scrollIntoView({ block: 'center' });
  await trigger.click();
  await clickWithText('[data-slot="dropdown-menu-item"]', label);
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

/** Sends with Enter and waits for the new turn, so reply checks cannot read the previous one. */
export async function sendMessage(text: string): Promise<void> {
  await waitForComposer();
  const before = await turnCount();
  await waitForPointerInput();
  const box = await $(COMPOSER);
  await box.click();
  await box.setValue(text);
  await browser.keys(Key.Enter);
  await waitForNewTurn(before);
}

/** How many turns the transcript shows. */
export async function turnCount(): Promise<number> {
  return browser.execute((turns: string) => document.querySelectorAll(turns).length, MAIN_TURNS);
}

/** The parts of the latest turn, in order. */
export async function latestTurnParts(): Promise<ShownPart[]> {
  return browser.execute((selector: string) => {
    const turns = document.querySelectorAll(selector);
    const latest = turns.item(turns.length - 1) as Element | null;
    if (latest === null) {
      return [];
    }
    return [...latest.querySelectorAll<HTMLElement>('[data-part-kind]')].map((element) => ({
      kind: element.dataset.partKind ?? '',
      state: element.dataset.partState ?? '',
      text: element.textContent.trim(),
    }));
  }, MAIN_TURNS);
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

/**
 * The conversations in the sidebar, by their links' titles. The activity
 * bar's Chat entry links to a conversation too, so only the content sidebar
 * counts.
 */
export async function listedConversations(): Promise<string[]> {
  return browser.execute(() =>
    [...document.querySelectorAll('[data-shell-sidebar="content"] a[href*="conversation="]')].map((link) =>
      link.textContent.trim(),
    ),
  );
}

/** A message that makes the tools scenario call one tool. */
export function callMessage(tool: string, args: Record<string, unknown>): string {
  return `call ${tool} ${JSON.stringify(args)}`;
}

/** Waits until the transcript shows more turns than `before`, so that later waits read the new turn. */
export async function waitForNewTurn(before: number): Promise<void> {
  await browser.waitUntil(async () => (await turnCount()) > before, {
    timeout: 10_000,
    timeoutMsg: 'the message did not show in the transcript',
  });
}

/**
 * Sends a message and waits until its turn shows. It counts the turns once
 * the message box takes messages, which is when a conversation that is still
 * loading, such as after leaving the settings, shows all its turns.
 */
export async function send(text: string): Promise<void> {
  await sendMessage(text);
}

/** The status of the latest tool call in the latest turn, or `null` when it has none. */
export async function latestToolStatus(): Promise<string | null> {
  return browser.execute((selector: string) => {
    const cards = document.querySelectorAll<HTMLElement>(selector);
    return cards[cards.length - 1]?.dataset.toolStatus ?? null;
  }, `${LATEST_MAIN_TURN} [data-tool-status]`);
}

/** Waits until the latest tool call of the latest turn has `status`. */
export async function waitForToolStatus(status: string, timeout = 30_000): Promise<void> {
  let last: string | null = null;
  await browser.waitUntil(
    async () => {
      last = await latestToolStatus();
      return last === status;
    },
    { timeout, timeoutMsg: `the latest tool call did not become ${status}; it is ${String(last)}` },
  );
}

/**
 * Clicks a button of the latest turn by its text, such as the approval's
 * "Allow once". The button moves to the middle of the transcript first, as
 * the owner would scroll to it, so that the button that jumps to the newest
 * message never covers it.
 */
export async function clickInLatestTurn(label: string): Promise<void> {
  const selector = `${LATEST_MAIN_TURN} button`;
  await browser.execute(
    (text: string, buttonSelector: string) => {
      const buttons = [...document.querySelectorAll<HTMLButtonElement>(buttonSelector)];
      buttons.find((button) => button.textContent.trim() === text)?.scrollIntoView({ block: 'center' });
    },
    label,
    selector,
  );
  await clickWithText(selector, label);
}

/** Sends a message of several lines, typed with Shift+Enter between them as the owner would, and waits until its turn shows. */
export async function sendLines(lines: readonly string[]): Promise<void> {
  await waitForComposer();
  const before = await turnCount();
  await waitForPointerInput();
  const box = await $(COMPOSER);
  await box.click();
  for (const [index, line] of lines.entries()) {
    if (index > 0) {
      await browser.keys([Key.Shift, Key.Enter]);
    }
    await box.addValue(line);
  }
  await browser.keys(Key.Enter);
  await waitForNewTurn(before);
}
