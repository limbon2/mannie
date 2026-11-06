// Questions to the owner in the conversation view (docs/features/questions.md,
// acceptance): a question waits in the transcript without taking the focus,
// the owner answers it from the keyboard or with the pointer, in their own
// words or not at all, the model reads the answer, and selections survive a
// switch to another conversation. The fixture provider's tools scenario calls
// the tool that each message names, and repeats the last result it read.
import { Key } from 'webdriverio';

import {
  addProfile,
  callMessage,
  clickInLatestTurn,
  COMPOSER,
  LATEST_MAIN_TURN,
  latestToolStatus,
  OPEN_CONVERSATION_ROW,
  send,
  startNewConversation,
  waitForCompleteReply,
  waitForToolStatus,
} from '../support/chat.ts';
import { deleteProviderProfiles } from '../support/harness.ts';
import { axeViolations, focusedElement, waitForAnimations, waitForApp, waitForShownText } from '../support/page.ts';
import { MODELS, type ProviderServer, startProviderServer, TOOLS_DONE } from '../support/provider-server.ts';

/** The card of the question that waits in the latest turn. */
const CARD = `${LATEST_MAIN_TURN} [data-question-card]`;

/** The icon that marks a conversation that waits for an answer. */
const MARK = 'svg[aria-label="Waiting for your answer"]';

/** That icon in the sidebar. */
const WAITING_MARK = `[data-slot="sidebar"] ${MARK}`;

const WHICH_DATABASE = {
  activity: 'Asking which database to use',
  questions: [
    {
      multiple: false,
      options: [
        { description: 'It has the JSON queries.', label: 'PostgreSQL' },
        { description: '', label: 'SQLite' },
      ],
      question: 'Which database should the service use?',
    },
  ],
};

const WHICH_EXTRAS = {
  questions: [
    {
      multiple: true,
      options: [
        { description: '', label: 'Caching' },
        { description: '', label: 'Metrics' },
      ],
      question: 'Which extras should it have?',
    },
    {
      multiple: false,
      options: [
        { description: '', label: 'Here' },
        { description: '', label: 'There' },
      ],
      question: 'Where should it run?',
    },
  ],
};

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

/** Whether the card's button that sends the answers can be pressed. */
async function canSend(): Promise<boolean> {
  return $(`${CARD} button[type="submit"]`).isEnabled();
}

/** Clicks the label of the card's option or choice with this text. */
async function choose(label: string): Promise<void> {
  await browser.execute(
    (card: string, text: string) => {
      const labels = [...document.querySelectorAll<HTMLElement>(`${card} label`)];
      labels.find((candidate) => candidate.textContent.trim() === text)?.click();
    },
    CARD,
    label,
  );
}

/** Which of the card's radio buttons are checked, by their labels. */
async function checkedRadios(): Promise<string[]> {
  return browser.execute((card: string) => {
    const radios = [...document.querySelectorAll(`${card} [role="radio"][aria-checked="true"]`)];
    return radios.map((radio) => document.querySelector(`label[for="${radio.id}"]`)?.textContent.trim() ?? '');
  }, CARD);
}

/** Moves the focus back with Shift+Tab until the element that `selector` matches has it. */
async function shiftTabUntilFocused(selector: string): Promise<void> {
  for (let presses = 0; presses < 30; presses += 1) {
    const reached = await browser.execute(
      (target: string) => document.activeElement?.matches(target) ?? false,
      selector,
    );
    if (reached) {
      return;
    }
    await browser.keys([Key.Shift, Key.Tab]);
  }
  throw new Error(`the focus did not reach ${selector}`);
}

/**
 * Moves through the radio group with the arrow key to the option with this
 * label, and chooses it with Space. An arrow key only moves the focus until the
 * group has a value, and the group wraps around.
 */
async function chooseWithKeys(label: string): Promise<void> {
  for (let presses = 0; presses < 5; presses += 1) {
    const focused = await browser.execute(() => {
      const id = document.activeElement?.id ?? '';
      return document.querySelector(`label[for="${id}"]`)?.textContent.trim() ?? '';
    });
    if (focused === label) {
      await browser.keys(Key.Space);
      return;
    }
    await browser.keys(Key.ArrowDown);
  }
  throw new Error(`the arrow key did not reach ${label}`);
}

/** Whether the focused element matches `selector`. */
async function focusMatches(selector: string): Promise<boolean> {
  return browser.execute((target: string) => document.activeElement?.matches(target) ?? false, selector);
}

describe('questions to the owner', () => {
  let provider: ProviderServer;

  before(async () => {
    provider = await startProviderServer();
    await waitForApp();
    await addProfile(provider.baseUrl, { model: MODELS.tools, name: 'Tools', useTools: true });
  });

  after(async () => {
    await deleteProviderProfiles();
    await provider.close();
  });

  it('waits for an answer without taking the focus, and says so in the status line and the sidebar', async () => {
    await send(callMessage('ask_owner', WHICH_DATABASE));
    await waitForToolStatus('awaiting_answer');
    await $(CARD).waitForDisplayed();

    await waitForShownText($('main'), expect.stringContaining('Waiting for your answer.'));
    const row = $(`${LATEST_MAIN_TURN} [data-tool-toggle]`);
    await waitForShownText(row, expect.stringContaining('Asking which database to use'));
    await waitForShownText(row, expect.stringContaining('Waiting for your answer'));
    await expect($(WAITING_MARK)).toBeExisting();
    expect(await focusMatches(COMPOSER)).toBe(true);
    expect(await canSend()).toBe(false);
    await axeInBothThemes();
  });

  it('takes the answer from the keyboard, gives it to the model, and returns the focus to the call', async () => {
    // The tab order runs back from the message box through the card's buttons
    // to the radio group, which holds one stop.
    await $(COMPOSER).click();
    await shiftTabUntilFocused(`${CARD} [role="radio"]`);
    await chooseWithKeys('SQLite');
    await browser.waitUntil(async () => (await checkedRadios()).includes('SQLite'), {
      timeoutMsg: 'Space did not choose SQLite',
    });
    await browser.keys(Key.Tab);
    expect(await focusedElement()).toEqual(expect.objectContaining({ label: 'Send answer', tag: 'button' }));
    await browser.keys(Key.Enter);

    const reply = await waitForCompleteReply();
    expect(reply).toContain(TOOLS_DONE);
    expect(reply).toContain('Which database should the service use?');
    expect(reply).toContain('Answer: SQLite');
    expect(await latestToolStatus()).toBe('completed');
    await expect($(CARD)).not.toBeExisting();
    await expect($(WAITING_MARK)).not.toBeExisting();
    const answers = $(`${LATEST_MAIN_TURN} [data-question-answers]`);
    await waitForShownText(answers, expect.stringContaining('Which database should the service use?'));
    await waitForShownText(answers, expect.stringContaining('SQLite'));
    expect(await focusMatches(`${LATEST_MAIN_TURN} [data-tool-toggle]`)).toBe(true);
    await axeInBothThemes();
  });

  it('takes several options and the owner’s own words for several questions', async () => {
    await startNewConversation();
    await send(callMessage('ask_owner', WHICH_EXTRAS));
    await waitForToolStatus('awaiting_answer');
    await $(CARD).waitForDisplayed();

    await choose('Metrics');
    await choose('Caching');
    expect(await canSend()).toBe(false);
    // In the second question, "Something else" asks for the words first.
    const somethingElse = await $$(`${CARD} [role="radio"]`);
    await somethingElse[2]?.click();
    expect(await canSend()).toBe(false);
    await $(`${CARD} textarea`).waitForDisplayed();
    await axeInBothThemes();
    await $(`${CARD} textarea`).setValue('On the build machine');
    expect(await canSend()).toBe(true);
    await clickInLatestTurn('Send answers');

    const reply = await waitForCompleteReply();
    expect(reply).toContain('Answer: Caching; Metrics');
    expect(reply).toContain("In the owner's own words: On the build machine");
    const answers = $(`${LATEST_MAIN_TURN} [data-question-answers]`);
    await waitForShownText(answers, expect.stringContaining('Caching and Metrics'));
    await waitForShownText(answers, expect.stringContaining('On the build machine'));
  });

  it('keeps the owner’s choices across a switch of conversations, and shows the question again after a reload', async () => {
    await startNewConversation();
    await send(callMessage('ask_owner', WHICH_DATABASE));
    await waitForToolStatus('awaiting_answer');
    await $(CARD).waitForDisplayed();
    await choose('PostgreSQL');
    await browser.waitUntil(async () => (await checkedRadios()).includes('PostgreSQL'));

    await startNewConversation();
    await expect($(WAITING_MARK)).toBeExisting();
    // Back through the sidebar: loading the address would start the window anew.
    await $(`[data-conversation]:has(${MARK}) a`).click();
    await $(CARD).waitForDisplayed();
    await waitForAnimations();
    expect(await checkedRadios()).toEqual(['PostgreSQL']);
    expect(await canSend()).toBe(true);

    await browser.refresh();
    await waitForApp();
    await $(CARD).waitForDisplayed({ timeout: 15_000 });
    expect(await latestToolStatus()).toBe('awaiting_answer');
    expect(await checkedRadios()).toEqual([]);
    expect(await canSend()).toBe(false);
  });

  it('tells the model when the owner chooses not to answer', async () => {
    await clickInLatestTurn("Don't answer");

    expect(await waitForCompleteReply()).toContain('The owner chose not to answer.');
    expect(await latestToolStatus()).toBe('completed');
    await waitForShownText($(`${LATEST_MAIN_TURN} [data-tool-toggle]`), expect.stringContaining('Not answered'));
    await expect($(CARD)).not.toBeExisting();
  });

  it('cancels the question when the owner stops the reply', async () => {
    await send(callMessage('ask_owner', WHICH_DATABASE));
    await waitForToolStatus('awaiting_answer');
    await $(CARD).waitForDisplayed();

    await $(COMPOSER).click();
    await browser.keys(Key.Escape);

    await waitForToolStatus('canceled');
    await expect($(CARD)).not.toBeExisting();
    await expect($(`${OPEN_CONVERSATION_ROW} ${MARK}`)).not.toBeExisting();
  });
});
