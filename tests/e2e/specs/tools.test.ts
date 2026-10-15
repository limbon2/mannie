// System tools in the conversation view (docs/features/system-tools/
// tool-activity.md, acceptance): in a conversation of a project, reads inside
// the project's folder run without a question, changes ask, the owner's
// answers reach the model, permissions and rules last as long as they should,
// Stop ends a running command, a waiting request survives a reload, and a
// command that was running when the app died is never run again. The fixture
// provider's tools scenario calls the tool that each message names.
import { existsSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';

import {
  addProfile,
  addProject,
  allowFor,
  callMessage,
  clickInLatestTurn,
  clickWithText,
  COMPOSER,
  latestToolStatus,
  projectRow,
  retryLatestReply,
  send,
  sendLines,
  startConversationIn,
  useProfile,
  waitForCompleteReply,
  waitForToolStatus,
} from '../support/chat.ts';
import { deleteProviderProfiles, processId } from '../support/harness.ts';
import {
  axeViolations,
  focusedElement,
  leaveSettings,
  openSettings,
  runCommand,
  waitForAnimations,
  waitForApp,
} from '../support/page.ts';
import { MODELS, type ProviderServer, startProviderServer, TOOLS_DONE } from '../support/provider-server.ts';

const IS_WINDOWS = process.platform === 'win32';

/** A command that waits for a minute, in the platform's shell. */
const WAIT_A_MINUTE = IS_WINDOWS ? 'Start-Sleep -Seconds 60' : 'sleep 60';

const DIALOG = '[data-slot="dialog-content"]';

/** When the owner pressed Escape and when the running call ended, in the page's clock. */
interface StopTiming {
  ended: number;
  keydown: number;
}

declare global {
  interface Window {
    toolStopTiming?: StopTiming;
  }
}

/** The lines of a message that makes each call a step of its own. */
function inSteps(calls: readonly string[]): string[] {
  return calls.flatMap((call, index) => (index === 0 ? [call] : ['next', call]));
}

/** Whether the latest turn's button with this text is inside the transcript's view. */
async function inTranscriptView(label: string): Promise<boolean> {
  return browser.execute((text: string) => {
    const transcript = document.querySelector('main [role="region"]');
    const buttons = [...document.querySelectorAll<HTMLButtonElement>('main article:last-of-type button')];
    const button = buttons.find((candidate) => candidate.textContent.trim() === text);
    if (transcript === null) {
      return false;
    }
    if (button === undefined) {
      return false;
    }
    const view = transcript.getBoundingClientRect();
    const target = button.getBoundingClientRect();
    const startsInView = target.top >= view.top;
    const endsInView = target.bottom <= view.bottom;
    return startsInView && endsInView;
  }, label);
}

/** Whether the transcript offers to jump back to its newest message. */
async function offersToJump(): Promise<boolean> {
  return browser.execute(() =>
    [...document.querySelectorAll('main button')].some((button) => button.textContent.includes('Jump to')),
  );
}

/**
 * The elements around the transcript that could scroll, with how far: none
 * should, because everything in a reply belongs to the transcript's own
 * scrolling, also the text that only screen readers read.
 */
async function overflowAroundTranscript(): Promise<string[]> {
  return browser.execute(() => {
    const overflowing: string[] = [];
    let element = document.querySelector('main [role="region"]')?.parentElement ?? null;
    while (element !== null) {
      const overflow = element.scrollHeight - element.clientHeight;
      if (overflow > 1) {
        overflowing.push(`${element.tagName.toLowerCase()}.${element.className}: ${String(overflow)} px`);
      }
      element = element.parentElement;
    }
    return overflowing;
  });
}

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

/** Replaces a field's text as the owner types it. */
async function fill(selector: string, value: string): Promise<void> {
  const field = await $(selector);
  await field.click();
  await field.clearValue();
  await field.setValue(value);
}

describe('system tools', () => {
  let provider: ProviderServer;
  let workspace: string;
  /** The project whose folder is the workspace. */
  let project: string;

  before(async () => {
    provider = await startProviderServer();
    // The folder as the system names it, which is how the core stores it.
    workspace = realpathSync.native(mkdtempSync(join(tmpdir(), 'mannie-e2e-tools-')));
    writeFileSync(join(workspace, 'notes.txt'), 'first line\nsecond line\n');
    await waitForApp();
    await addProfile(provider.baseUrl, { model: MODELS.tools, name: 'Tools', useTools: true });
    await addProfile(provider.baseUrl, {
      interface: 'responses',
      model: MODELS.tools,
      name: 'Tools on Responses',
      useTools: true,
    });
    await addProfile(provider.baseUrl, { model: MODELS.echo, name: 'Without tools' });
  });

  after(async () => {
    // The profiles' credentials live in the system's credential store.
    await deleteProviderProfiles();
    await provider.close();
    rmSync(workspace, { force: true, maxRetries: 20, recursive: true, retryDelay: 250 });
  });

  it('makes the folder chosen in the folder dialog a project, which opens with a new conversation', async () => {
    project = await addProject(workspace);

    await expect(projectRow(project)).toHaveText(expect.stringContaining(basename(workspace)));
    expect(await axeViolations()).toEqual([]);
  });

  it('reads inside the project’s folder without asking, through both APIs', async () => {
    await send(callMessage('read_file', { path: 'notes.txt' }));

    const reply = await waitForCompleteReply();
    expect(reply).toContain(TOOLS_DONE);
    expect(reply).toContain('first line');
    expect(await latestToolStatus()).toBe('completed');
    // The conversation started in the project, and the sidebar lists it there.
    await expect(projectRow(project).$('[data-conversation]')).toBeExisting();

    await useProfile('Tools on Responses');
    await send(callMessage('read_file', { path: 'notes.txt' }));

    expect(await waitForCompleteReply()).toContain('second line');
    await useProfile('Tools');
  });

  it('shows a call as one row with the activity that the model named, and its details on demand', async () => {
    await send(callMessage('read_file', { activity: 'Reading the notes', path: 'notes.txt' }));
    await waitForCompleteReply();

    const row = $('main article:last-of-type [data-tool-toggle]');
    await expect(row).toHaveText(expect.stringContaining('Reading the notes'));
    // The line that Mannie built names the file; the row leaves it to the details.
    await expect(row).not.toHaveText(expect.stringContaining('notes.txt'));
    await row.click();
    await waitForAnimations();

    const call = $('main article:last-of-type [data-tool-status]');
    await expect(call).toHaveText(expect.stringContaining('lines 1 to 2 of 2'));
    await expect(call).toHaveText(expect.stringContaining('notes.txt'));
    await axeInBothThemes();
  });

  it('keeps a request that follows many calls in view, without the owner scrolling', async () => {
    await startConversationIn(project);
    const looks = (count: number): string[] =>
      Array.from({ length: count }, (_, step) =>
        callMessage('list_directory', { activity: `Looking around, step ${String(step + 1)}`, path: '.' }),
      );
    // A first reply fills the transcript past its height. When the long
    // message that follows leaves the message box, the box shrinks, and the
    // transcript grows taller and moves up while the new turn grows.
    await sendLines(inSteps(looks(8)));
    await waitForCompleteReply();
    const write = callMessage('write_file', { activity: 'Writing the summary', content: 'done', path: 'summary.txt' });
    await sendLines(inSteps([...looks(12), write]));
    await waitForToolStatus('awaiting_approval');
    await waitForAnimations();

    // The transcript follows the request into view; the window of the tests
    // renders its frames slowly while it sits off the screen.
    await browser.waitUntil(async () => inTranscriptView('Allow once'), {
      timeout: 5_000,
      timeoutMsg: 'the request did not come into view',
    });
    expect(await offersToJump()).toBe(false);
    expect(await overflowAroundTranscript()).toEqual([]);
    await clickInLatestTurn('Allow once');

    expect(await waitForCompleteReply()).toContain(TOOLS_DONE);
    expect(readFileSync(join(workspace, 'summary.txt'), 'utf8')).toBe('done');
  });

  it('asks before a change without taking the focus, and runs the change once allowed', async () => {
    await send(callMessage('write_file', { content: 'hello', path: 'created.txt' }));
    await waitForToolStatus('awaiting_approval');

    await expect($('main')).toHaveText(expect.stringContaining('Waiting for your approval.'));
    await expect($('[data-slot="sidebar"] svg[aria-label="Waiting for your approval"]')).toBeExisting();
    expect(await focusedElement()).toEqual(expect.objectContaining({ tag: 'textarea' }));
    await axeInBothThemes();

    await clickInLatestTurn('Allow once');

    expect(await waitForCompleteReply()).toContain(TOOLS_DONE);
    expect(readFileSync(join(workspace, 'created.txt'), 'utf8')).toBe('hello');
    await expect($('[data-slot="sidebar"] svg[aria-label="Waiting for your approval"]')).not.toBeExisting();
  });

  it('tells the model that the owner denied a change, and changes nothing', async () => {
    await send(callMessage('delete_path', { path: 'notes.txt' }));
    await waitForToolStatus('awaiting_approval');

    await clickInLatestTurn('Deny');

    expect(await waitForCompleteReply()).toContain('The owner denied this operation.');
    expect(await latestToolStatus()).toBe('denied');
    expect(existsSync(join(workspace, 'notes.txt'))).toBe(true);
  });

  it('allows the same kind of change for the rest of the conversation, until the owner revokes it', async () => {
    await send(callMessage('write_file', { content: '1', path: 'one.txt' }));
    await waitForToolStatus('awaiting_approval');
    await allowFor('This conversation');
    await waitForCompleteReply();

    await send(callMessage('write_file', { content: '2', path: 'two.txt' }));
    expect(await waitForCompleteReply()).toContain(TOOLS_DONE);
    expect(readFileSync(join(workspace, 'two.txt'), 'utf8')).toBe('2');

    await runCommand('Show the permissions of this conversation');
    const dialog = await $(DIALOG);
    const grant = `Creating files and folders in ${workspace}`;
    await expect(dialog).toHaveText(expect.stringContaining(grant));
    await waitForAnimations();
    expect(await axeViolations()).toEqual([]);
    await clickWithText(`${DIALOG} button`, 'Revoke');
    await expect(dialog).not.toHaveText(expect.stringContaining(grant));
    await clickWithText(`${DIALOG} button`, 'Close');
    await dialog.waitForDisplayed({ reverse: true });

    await send(callMessage('write_file', { content: '3', path: 'three.txt' }));
    await waitForToolStatus('awaiting_approval');
    await clickInLatestTurn('Deny');
    await waitForCompleteReply();
    expect(existsSync(join(workspace, 'three.txt'))).toBe(false);
  });

  it('saves a rule from a command, which then covers the command and which the settings list', async () => {
    await send(callMessage('run_command', { command: 'echo rule-test' }));
    await waitForToolStatus('awaiting_approval');
    await clickInLatestTurn('Save rule…');
    const dialog = await $(DIALOG);
    await dialog.waitForDisplayed();
    await waitForAnimations();
    expect(await axeViolations()).toEqual([]);
    await $('#tools-rule-words-1').click();
    await clickWithText(`${DIALOG} button`, 'Save rule and allow');
    await dialog.waitForDisplayed({ reverse: true });
    expect(await waitForCompleteReply()).toContain('rule-test');

    await send(callMessage('run_command', { command: 'echo covered' }));
    expect(await waitForCompleteReply()).toContain('covered');
    expect(await latestToolStatus()).toBe('completed');

    await openSettings('tools.settings');
    await expect($('main')).toHaveText(expect.stringContaining(`In ${workspace} and the folders inside it`));
    await clickWithText('main button', 'Delete');
    await expect($('main')).toHaveText(expect.stringContaining('No rules yet'));
    await leaveSettings();
  });

  it('ends a running command within a second when the owner stops the reply', async () => {
    await send(callMessage('run_command', { command: WAIT_A_MINUTE }));
    await waitForToolStatus('awaiting_approval');
    await clickInLatestTurn('Allow once');
    await waitForToolStatus('executing');
    await expect($('main')).toHaveText(expect.stringContaining('Running a command…'));
    await browser.execute(() => {
      const timing: StopTiming = { ended: 0, keydown: 0 };
      window.toolStopTiming = timing;
      document.addEventListener(
        'keydown',
        (event) => {
          if (event.key === 'Escape') {
            timing.keydown = performance.now();
          }
        },
        { capture: true },
      );
      const main = document.querySelector('main');
      if (main === null) {
        throw new Error('the page has no main area');
      }
      const observer = new MutationObserver(() => {
        const cards = main.querySelectorAll<HTMLElement>('article:last-of-type [data-tool-status]');
        const status = cards[cards.length - 1]?.dataset.toolStatus;
        if (status !== 'executing') {
          timing.ended = performance.now();
          observer.disconnect();
        }
      });
      observer.observe(main, { attributeFilter: ['data-tool-status'], attributes: true, subtree: true });
    });

    await $(COMPOSER).click();
    await browser.keys('Escape');

    await waitForToolStatus('completed', 5_000);
    const timing = await browser.execute(() => window.toolStopTiming);
    expect(timing?.ended).toBeGreaterThan(0);
    expect((timing?.ended ?? 0) - (timing?.keydown ?? 0)).toBeLessThan(1_000);
    await expect($('main')).toHaveText(expect.stringContaining('You stopped this reply.'));
    await expect($('main article:last-of-type [data-tool-status]')).toHaveText(
      expect.stringContaining('stopped after'),
    );
  });

  it('shows a waiting request again after the window reloads', async () => {
    await send(callMessage('write_file', { content: 'after reload', path: 'reloaded.txt' }));
    await waitForToolStatus('awaiting_approval');

    await browser.refresh();
    await waitForApp();

    await waitForToolStatus('awaiting_approval');
    await clickInLatestTurn('Allow once');
    expect(await waitForCompleteReply()).toContain(TOOLS_DONE);
    expect(readFileSync(join(workspace, 'reloaded.txt'), 'utf8')).toBe('after reload');
  });

  it('archives a long output, and shows all of it page by page', async () => {
    const command = IS_WINDOWS ? '1..10000 | ForEach-Object { "line $_" }' : "seq 1 10000 | sed 's/^/line /'";
    await send(callMessage('run_command', { command }));
    await waitForToolStatus('awaiting_approval');
    await clickInLatestTurn('Allow once');
    await waitForCompleteReply(60_000);

    await $('main article:last-of-type [data-tool-toggle]').click();
    await clickInLatestTurn('Show full output');
    const dialog = await $(DIALOG);
    await expect(dialog).toHaveText(expect.stringContaining('Showing 500 of'));
    await expect(dialog).toHaveText(expect.stringContaining('line 1'));
    await waitForAnimations();
    expect(await axeViolations()).toEqual([]);
    await clickWithText(`${DIALOG} button`, 'Load more');

    await expect(dialog).toHaveText(expect.stringContaining('Showing 1,000 of'));
    await clickWithText(`${DIALOG} button`, 'Close');
    await dialog.waitForDisplayed({ reverse: true });
  });

  it('runs changes inside the workspace without asking once the settings allow it', async () => {
    await openSettings('tools.settings');
    await axeInBothThemes();
    await $('#tools-allow-changes').click();
    await expect($('#tools-allow-changes')).toHaveAttribute('aria-checked', 'true');
    await fill('#tools-environment', 'GITHUB_TOKEN');
    await clickWithText('main button', 'Save');
    await expect($('main')).toHaveText(
      expect.stringContaining('GITHUB_TOKEN looks like the name of a secret, which commands never receive.'),
    );
    await fill('#tools-environment', 'MANNIE_E2E_VALUE');
    await clickWithText('main button', 'Save');
    await expect($('main')).toHaveText(expect.stringContaining('Saved'));
    await leaveSettings();

    await send(callMessage('write_file', { content: 'free', path: 'free.txt' }));

    expect(await waitForCompleteReply()).toContain(TOOLS_DONE);
    expect(readFileSync(join(workspace, 'free.txt'), 'utf8')).toBe('free');
    await openSettings('tools.settings');
    await $('#tools-allow-changes').click();
    await expect($('#tools-allow-changes')).toHaveAttribute('aria-checked', 'false');
    await leaveSettings();
  });

  it('sends no tools for a profile that does not use them', async () => {
    await useProfile('Without tools');

    await send('Plain chat');

    expect(await waitForCompleteReply()).toBe('Reply to: Plain chat');
    const call = provider
      .requests()
      .filter((request) => request.body?.model === MODELS.echo)
      .at(-1);
    expect(call?.body?.tools).toBeUndefined();
    await useProfile('Tools');
  });

  it('marks a command that ran when the app died as "May have run", and Retry does not run it again', async () => {
    const marker = join(workspace, 'marker.txt');
    const command = IS_WINDOWS
      ? `Add-Content -Path marker.txt -Value ran; ${WAIT_A_MINUTE}`
      : `echo ran >> marker.txt; ${WAIT_A_MINUTE}`;
    await send(callMessage('run_command', { command }));
    await waitForToolStatus('awaiting_approval');
    await clickInLatestTurn('Allow once');
    await waitForToolStatus('executing');
    await browser.waitUntil(() => existsSync(marker), { timeout: 15_000, timeoutMsg: 'the command did not start' });
    const conversationUrl = await browser.getUrl();

    process.kill(await processId(), 'SIGKILL');
    await browser.reloadSession();
    await waitForApp();
    await browser.url(conversationUrl);
    await waitForApp();

    await waitForToolStatus('uncertain');
    await expect($('main article:last-of-type [data-tool-status]')).toHaveText(expect.stringContaining('May have run'));
    await retryLatestReply();

    expect(await waitForCompleteReply()).toContain(TOOLS_DONE);
    expect(readFileSync(marker, 'utf8').trim().split(/\r?\n/u)).toEqual(['ran']);
  });
});
