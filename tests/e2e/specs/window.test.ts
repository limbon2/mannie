// The window and the tray (docs/features/window-and-tray.md, acceptance): the
// first close asks once and the answer lasts; in tray mode closing hides the
// window while a reply keeps streaming; in quit mode closing with a reply
// running warns, Cancel keeps it running, and quitting leaves it interrupted
// and retryable; the tray's Quit warns the same way; and starting Mannie
// again shows its window. WebDriver cannot reach the close button or the
// tray, so validation commands run the same code (the feature's tests section).
import { spawn } from 'node:child_process';

import { applicationPath } from '../support/app.ts';
import {
  addProfile,
  clickWithText,
  retryLatestReply,
  sendMessage,
  startNewConversation,
  waitForCompleteReply,
  waitForLatestPart,
} from '../support/chat.ts';
import { chooseTrayItem, closeMainWindow, deleteProviderProfiles, processId, windowState } from '../support/harness.ts';
import { axeViolations, openSettings, waitForAnimations, waitForApp } from '../support/page.ts';
import { MODELS, pacedText, type ProviderServer, startProviderServer } from '../support/provider-server.ts';

const QUESTION = '[data-slot="dialog-content"]';
const WARNING = '[data-slot="alert-dialog-content"]';

/** Whether the process with this ID still runs. */
function isRunning(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

/** Waits for `condition` in this process, without the WebDriver session, which ends when Mannie quits. */
async function poll(condition: () => boolean, timeoutMillis: number, message: string): Promise<void> {
  const deadline = Date.now() + timeoutMillis;
  while (!condition()) {
    if (Date.now() > deadline) {
      throw new Error(message);
    }
    await new Promise((resolve) => {
      setTimeout(resolve, 250);
    });
  }
}

async function waitForVisibility(visible: boolean): Promise<void> {
  await browser.waitUntil(async () => (await windowState()).visible === visible, {
    timeoutMsg: visible ? 'the window did not show' : 'the window did not hide',
  });
}

/** Opens the window settings and chooses what closing the window does. */
async function chooseCloseAction(action: 'ask' | 'quit' | 'tray'): Promise<void> {
  await openSettings('window.behavior');
  const choice = await $(`#window-close-${action}`);
  await choice.click();
  await expect(choice).toHaveAttribute('data-state', 'checked');
  // The radio updates before the saved close action reaches the core.
  await expect(choice).toBeEnabled();
}

/** Waits until a process ends, and kills it when it does not end in time. */
async function waitForExit(child: ReturnType<typeof spawn>, timeoutMillis: number): Promise<void> {
  await new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => {
      child.kill();
      reject(new Error('the process did not end'));
    }, timeoutMillis);
    child.on('exit', () => {
      clearTimeout(timer);
      resolve();
    });
  });
}

/** Sends a message to the paced model and waits until its reply streams. */
async function startPacedReply(text: string): Promise<void> {
  await startNewConversation();
  await sendMessage(text);
  await waitForLatestPart('text', ['streaming']);
}

describe('window and tray', () => {
  let provider: ProviderServer;

  before(async () => {
    provider = await startProviderServer();
    await waitForApp();
    await addProfile(provider.baseUrl, { model: MODELS.paced, name: 'Paced' });
  });

  after(async () => {
    // The profiles' credentials live in the system's credential store.
    await deleteProviderProfiles();
    await provider.close();
  });

  it('creates the tray with the labels that the window gives it', async () => {
    await browser.waitUntil(async () => (await windowState()).tray, { timeoutMsg: 'the tray was not created' });
    expect(await windowState()).toEqual({ tray: true, visible: true });
  });

  it('asks on the first close, then keeps the reply streaming in the tray', async () => {
    await startPacedReply('Count while hidden');

    await closeMainWindow();

    const question = await $(QUESTION);
    await question.waitForDisplayed();
    await waitForAnimations();
    await expect(question).toHaveText(expect.stringContaining('What should closing the window do?'));
    await expect(question).toHaveText(expect.stringContaining('1 reply is being written.'));
    expect(await axeViolations()).toEqual([]);
    await clickWithText(`${QUESTION} button`, 'Keep running in the tray');
    await waitForVisibility(false);

    await chooseTrayItem('open');
    await waitForVisibility(true);
    expect(await waitForCompleteReply()).toBe(pacedText());
  });

  it('remembers the answer: the next close hides the window without a question', async () => {
    await closeMainWindow();
    await waitForVisibility(false);

    await chooseTrayItem('open');
    await waitForVisibility(true);
    await expect($(QUESTION)).not.toBeExisting();
  });

  it('keeps the answer across a restart, and the settings show it', async () => {
    await browser.reloadSession();
    await waitForApp();

    await openSettings('window.behavior');
    await expect($('#window-close-tray')).toHaveAttribute('data-state', 'checked');
  });

  it('warns before quitting while a reply runs, and Cancel keeps it running', async () => {
    await chooseCloseAction('quit');
    await startPacedReply('Count through a warning');

    await closeMainWindow();

    const warning = await $(WARNING);
    await warning.waitForDisplayed();
    await waitForAnimations();
    await expect(warning).toHaveText(expect.stringContaining('Quit while replies are being written?'));
    expect(await axeViolations()).toEqual([]);
    await clickWithText(`${WARNING} button`, 'Cancel');
    await warning.waitForDisplayed({ reverse: true });
    expect(await windowState()).toEqual({ tray: true, visible: true });
    expect(await waitForCompleteReply()).toBe(pacedText());
  });

  it('warns the same way when the tray menu quits while a reply runs', async () => {
    await startPacedReply('Count through the tray');

    await chooseTrayItem('quit');

    const warning = await $(WARNING);
    await warning.waitForDisplayed();
    await waitForAnimations();
    await clickWithText(`${WARNING} button`, 'Cancel');
    await warning.waitForDisplayed({ reverse: true });
    expect(await waitForCompleteReply()).toBe(pacedText());
  });

  it('quits when the owner confirms, leaving the reply interrupted and retryable', async () => {
    await startPacedReply('Count until quitting');
    const conversationUrl = await browser.getUrl();
    const pid = await processId();

    await closeMainWindow();
    const warning = await $(WARNING);
    await warning.waitForDisplayed();
    await waitForAnimations();
    await clickWithText(`${WARNING} button`, 'Quit anyway');
    await poll(() => !isRunning(pid), 15_000, 'Mannie did not quit');

    await browser.reloadSession();
    await waitForApp();
    await browser.url(conversationUrl);
    await waitForApp();
    const interrupted = await waitForLatestPart('text', ['interrupted']);
    expect(pacedText().startsWith(interrupted.text)).toBe(true);
    await retryLatestReply();
    expect(await waitForCompleteReply()).toBe(pacedText());
  });

  it('shows the window when Mannie starts again while it runs in the tray', async () => {
    await chooseCloseAction('tray');
    await closeMainWindow();
    await waitForVisibility(false);

    // The second instance hands over to the first one and ends.
    const second = spawn(applicationPath(), [], { stdio: 'ignore' });
    const exited = waitForExit(second, 15_000);

    await waitForVisibility(true);
    await exited;
  });
});
