// Desktop updates (docs/features/updates.md, acceptance; ADR-0063): the
// settings section shows the running version, and the switch for automatic
// checks keeps its position across a restart; a version that is ready shows in
// the section and in a notice; "Restart and update" while a reply is being
// written asks first, and the confirmation records one install; a notice says
// how the last install ended, and closes. A validation build reads a fake
// source that this spec sets, sends no request and records an install in place
// of quitting (ADR-0063, decision 5), so V12 checks a real update by hand.
import { Key } from 'webdriverio';

import { addProfile, sendMessage, startNewConversation, waitForLatestPart } from '../support/chat.ts';
import { deleteProviderProfiles, setUpdateOffer, setUpdateOutcome, updateInstalls } from '../support/harness.ts';
import {
  axeViolations,
  focusedElement,
  focusIsVisible,
  isDark,
  openSettings,
  runCommand,
  switchToPseudoLocale,
  uncataloguedText,
  waitForApp,
  waitForGone,
  waitForShownText,
} from '../support/page.ts';
import { MODELS, type ProviderServer, startProviderServer } from '../support/provider-server.ts';

const SETTINGS_SECTION = 'updates.settings';
const SECTION = '[data-updates-section]';
const READY_STATE = '[data-updates-state="ready"]';
const AUTOMATIC = '#updates-check-automatically';
const READY_NOTICE = '[data-update-notice="ready"]';
const OUTCOME_NOTICE = '[data-update-notice="outcome"]';
const CLOSE_NOTICE = '[data-update-notice-close]';
const QUESTION = '[data-update-question]';
const CONFIRM = '[data-update-question-confirm]';

/** A version above the running one, which the fake source offers. */
const NEWER = '9.9.9';
/** The version that an install started from, in the result of an install. */
const OLDER = '0.0.1';

/** Puts the focus on an element, so that the keys that follow go to it. */
async function focus(selector: string): Promise<void> {
  await browser.execute((chosen: string) => document.querySelector<HTMLElement>(chosen)?.focus(), selector);
}

/**
 * Presses Tab until the focus is on an element. A dialog keeps the focus
 * inside, so a few presses reach each of its buttons.
 */
async function tabTo(selector: string): Promise<void> {
  await browser.waitUntil(
    async () => {
      await browser.keys(Key.Tab);
      return $(selector).isFocused();
    },
    { timeoutMsg: `Tab did not reach ${selector}` },
  );
}

/** Lets the fake source offer the newer version, checks, and waits until it is ready. */
async function findNewerVersion(): Promise<void> {
  await setUpdateOffer({ kind: 'version', version: NEWER });
  await runCommand('Check for updates');
  await $(READY_STATE).waitForDisplayed();
}

/** Closes the notice of the version that is ready, which stays ready. */
async function closeReadyNotice(): Promise<void> {
  await $(`${READY_NOTICE} ${CLOSE_NOTICE}`).click();
  await waitForGone(READY_NOTICE);
}

/**
 * Starts Mannie again as after an install that ended with `result`, and waits
 * for the notice. The tests cannot install a version, so the harness sets the
 * result that a start after an install finds in the journal.
 */
async function startAfterInstall(result: 'completed' | 'failed'): Promise<void> {
  await browser.reloadSession();
  await waitForApp();
  await setUpdateOutcome({ fromVersion: OLDER, result, toVersion: NEWER });
  await $(`${OUTCOME_NOTICE}[data-update-result="${result}"]`).waitForDisplayed();
}

describe('desktop updates', () => {
  let provider: ProviderServer;

  before(async () => {
    provider = await startProviderServer();
    await waitForApp();
    // This model starts a reply and never ends it, so the reply is still being
    // written whenever the question about the restart shows.
    await addProfile(provider.baseUrl, { model: MODELS.hang, name: 'Hang' });
  });

  after(async () => {
    // A profile cannot be deleted while a reply uses it, and the last test's
    // reply ends with the app.
    await browser.reloadSession();
    await waitForApp();
    // The profiles' credentials live in the system's credential store.
    await deleteProviderProfiles();
    await provider.close();
  });

  it('shows the running version before a check', async () => {
    await openSettings(SETTINGS_SECTION);
    await $(SECTION).waitForDisplayed();

    await waitForShownText('[data-updates-running-version]', /Mannie \d+\.\d+\.\d+ is running\./u);
    await $('[data-updates-state="idle"]').waitForDisplayed();
    await waitForShownText(
      '[data-updates-status]',
      expect.stringContaining('Mannie has not checked for updates since it started.'),
    );
    await $(`${AUTOMATIC}[data-state="checked"]`).waitForDisplayed();
    expect(await axeViolations()).toEqual([]);
  });

  it('keeps the switch for automatic checks across a restart', async () => {
    await focus(AUTOMATIC);
    await browser.keys(Key.Space);
    await $(`${AUTOMATIC}[data-state="unchecked"]`).waitForDisplayed();
    // The switch is disabled while the core saves the setting.
    await $(AUTOMATIC).waitForEnabled();

    await browser.reloadSession();
    await waitForApp();
    await openSettings(SETTINGS_SECTION);
    await $(`${AUTOMATIC}[data-state="unchecked"]`).waitForDisplayed();
  });

  it('shows a version that is ready in the section and in a notice', async () => {
    await findNewerVersion();

    await waitForShownText('[data-updates-ready]', expect.stringContaining(`Mannie ${NEWER} is ready to install.`));
    await $(READY_NOTICE).waitForDisplayed();
    await waitForShownText(READY_NOTICE, expect.stringContaining(`Mannie ${NEWER} is ready to install`));
    expect(await axeViolations()).toEqual([]);
  });

  it('shows the section and the notice in both themes', async () => {
    await runCommand('Use the light theme');
    expect(await isDark()).toBe(false);
    expect(await axeViolations()).toEqual([]);

    await runCommand('Use the dark theme');
    expect(await isDark()).toBe(true);
    expect(await axeViolations()).toEqual([]);
  });

  it('asks before a restart that interrupts a reply, and records the install', async () => {
    // "Restart and update" stays in the command palette after the notice closed.
    await closeReadyNotice();
    await startNewConversation();
    await sendMessage('Count while Mannie updates');
    await waitForLatestPart('text', ['streaming']);

    await runCommand('Restart and update');
    await $(QUESTION).waitForDisplayed();
    await waitForShownText(QUESTION, expect.stringContaining('Restart while replies are being written?'));
    await waitForShownText(
      '[data-update-question-count="active-runs"]',
      expect.stringContaining('1 reply is being written.'),
    );
    expect(await axeViolations()).toEqual([]);
    expect(await updateInstalls()).toEqual([]);

    await tabTo(CONFIRM);
    expect(await focusIsVisible()).toBe(true);
    await browser.keys(Key.Enter);

    await $(QUESTION).waitForDisplayed({ reverse: true });
    expect(await updateInstalls()).toEqual([NEWER]);
  });

  it('says after a start that the last install worked, in a notice that closes', async () => {
    // The reply of the last test ends with the app.
    await startAfterInstall('completed');
    await waitForShownText(OUTCOME_NOTICE, expect.stringContaining(`Mannie was updated to ${NEWER}`));
    await runCommand('Use the dark theme');
    expect(await isDark()).toBe(true);
    expect(await axeViolations()).toEqual([]);
    await runCommand('Use the light theme');
    expect(await isDark()).toBe(false);
    expect(await axeViolations()).toEqual([]);
    await $(`${OUTCOME_NOTICE} ${CLOSE_NOTICE}`).click();
    await waitForGone(OUTCOME_NOTICE);
  });

  it('says after a start that the last install failed, in a notice that closes from the keyboard', async () => {
    // A closed notice stays closed in its window, so this result needs a start of its own.
    await startAfterInstall('failed');
    await waitForShownText(OUTCOME_NOTICE, expect.stringContaining(`Mannie ${NEWER} was not installed`));
    expect(await axeViolations()).toEqual([]);
    await runCommand('Use the dark theme');
    expect(await isDark()).toBe(true);
    expect(await axeViolations()).toEqual([]);

    await focus(`${OUTCOME_NOTICE} ${CLOSE_NOTICE}`);
    expect(await focusedElement()).toEqual(expect.objectContaining({ label: 'Close this notice', tag: 'button' }));
    await browser.keys(Key.Enter);
    await waitForGone(OUTCOME_NOTICE);
  });

  it('takes every text from the catalog, and shows the question in the light theme', async () => {
    await findNewerVersion();
    await closeReadyNotice();
    await startNewConversation();
    await sendMessage('Count in another language');
    await waitForLatestPart('text', ['streaming']);
    // The question showed in the dark theme in the test of the restart.
    await runCommand('Use the light theme');
    expect(await isDark()).toBe(false);

    // From here on, the spec finds elements by their structure alone. The
    // view loads anew with the locale, so the closed notice shows again.
    await switchToPseudoLocale();
    await setUpdateOutcome({ fromVersion: OLDER, result: 'failed', toVersion: NEWER });
    await $(OUTCOME_NOTICE).waitForDisplayed();
    await $(READY_NOTICE).waitForDisplayed();
    await $('[data-update-notice-restart]').click();
    await $(QUESTION).waitForDisplayed();
    expect(await uncataloguedText()).toEqual([]);
    expect(await axeViolations()).toEqual([]);

    await browser.keys(Key.Escape);
    await $(QUESTION).waitForDisplayed({ reverse: true });
    expect(await updateInstalls()).toEqual([]);

    await openSettings(SETTINGS_SECTION);
    await $(READY_STATE).waitForDisplayed();
    expect(await uncataloguedText()).toEqual([]);
    expect(await axeViolations()).toEqual([]);
  });
});
