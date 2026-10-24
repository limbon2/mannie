// Mentions in the message box (docs/features/chat/mentions.md, acceptance):
// "@" lists a project's files and folders, hidden and ignored ones included,
// a file that may hold secrets shows a warning, the keyboard goes into a
// folder and inserts a file, the message's chips and the transcript show the
// mentions, the model's request holds their text, a name with a space is
// found as the owner types it, Escape closes the list before it stops a
// reply, and a general conversation lists nothing.
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { Key } from 'webdriverio';

import {
  addProfile,
  addProject,
  COMPOSER,
  latestTurnParts,
  startNewConversation,
  turnCount,
  useProfile,
  waitForCompleteReply,
  waitForComposer,
  waitForLatestPart,
  waitForNewTurn,
} from '../support/chat.ts';
import { deleteProviderProfiles } from '../support/harness.ts';
import { axeViolations, waitForApp } from '../support/page.ts';
import { MODELS, type ProviderServer, startProviderServer } from '../support/provider-server.ts';

const OPTION = '[data-mention-list] [role="option"]';

/** The labels of the list's options, in order. */
async function optionLabels(): Promise<string[]> {
  return browser.execute(
    (selector: string) =>
      [...document.querySelectorAll(selector)].map(
        (option) => option.querySelector('[data-content]')?.textContent.trim() ?? '',
      ),
    OPTION,
  );
}

/** Waits until the list shows an option with `label`. */
async function waitForOption(label: string): Promise<void> {
  await browser.waitUntil(async () => (await optionLabels()).includes(label), {
    timeout: 10_000,
    timeoutMsg: `the list did not show ${label}`,
  });
}

/** Whether the option with `label` shows a warning. */
async function warns(label: string): Promise<boolean> {
  return browser.execute(
    (selector: string, wanted: string) =>
      [...document.querySelectorAll(selector)].some((option) => {
        const isWanted = option.querySelector('[data-content]')?.textContent.trim() === wanted;
        const hasWarning = option.querySelector('[data-mention-warning]') !== null;
        return isWanted && hasWarning;
      }),
    OPTION,
    label,
  );
}

/** Whether the list shows. */
async function listShows(): Promise<boolean> {
  return browser.execute(() => document.querySelector('[data-mention-list]') !== null);
}

/** Types at the end of the message box, key by key, as the owner does. */
async function type(text: string): Promise<void> {
  await waitForComposer();
  await $(COMPOSER).click();
  await browser.keys([Key.Ctrl, Key.End]);
  await browser.keys(text);
}

/** Empties the message box. */
async function clear(): Promise<void> {
  await $(COMPOSER).click();
  await browser.keys([Key.Ctrl, 'a']);
  await browser.keys(Key.Backspace);
  await browser.waitUntil(async () => (await $(COMPOSER).getValue()) === '', {
    timeoutMsg: 'the message box did not empty',
  });
}

/** The labels of the chips below the message box. */
async function chipLabels(): Promise<string[]> {
  return browser.execute(() =>
    [...document.querySelectorAll('[data-mention-chips] [data-content]')].map((chip) => chip.textContent.trim()),
  );
}

/** The text of the owner's latest message in the newest request to the model. */
function latestOwnerMessage(provider: ProviderServer): string {
  const requests = provider.requests().filter((request) => request.body !== null);
  const body = requests[requests.length - 1]?.body ?? {};
  const messages = (body.messages ?? []) as { content?: unknown; role?: string }[];
  const owner = messages.filter((message) => message.role === 'user').pop();
  return typeof owner?.content === 'string' ? owner.content : '';
}

describe('mentions', () => {
  let provider: ProviderServer;
  let folders: string;

  before(async () => {
    provider = await startProviderServer();
    // The folders as the system names them, which is how the core stores them.
    folders = realpathSync.native(mkdtempSync(join(tmpdir(), 'mannie-e2e-mentions-')));
    const folder = join(folders, 'orbit');
    mkdirSync(join(folder, 'src'), { recursive: true });
    mkdirSync(join(folder, 'docs'), { recursive: true });
    mkdirSync(join(folder, 'build'), { recursive: true });
    writeFileSync(join(folder, 'src', 'main.rs'), 'fn main() { println!("orbit"); }\n');
    writeFileSync(join(folder, 'docs', 'guide.md'), '# Guide\n');
    writeFileSync(join(folder, 'docs', 'release plan.md'), '# Release plan\n');
    writeFileSync(join(folder, '.env'), 'TOKEN=e2e\n');
    writeFileSync(join(folder, '.gitignore'), 'build/\n');
    writeFileSync(join(folder, 'build', 'out.log'), 'built\n');
    await waitForApp();
    await addProfile(provider.baseUrl, { model: MODELS.echo, name: 'Echo' });
    await addProfile(provider.baseUrl, { model: MODELS.hang, name: 'Hang' });
    await addProject(folder);
  });

  after(async () => {
    await deleteProviderProfiles();
    await provider.close();
    rmSync(folders, { force: true, maxRetries: 20, recursive: true, retryDelay: 250 });
  });

  it('lists the project’s files with hidden and ignored ones, and warns about a file that may hold secrets', async () => {
    await type('@');
    // Ignored entries come last, so they show once the list shows them all.
    await waitForOption('build/');

    expect(await optionLabels()).toEqual(expect.arrayContaining(['.env', '.gitignore', 'build/', 'docs/', 'src/']));
    expect(await warns('.env')).toBe(true);
    expect(await warns('.gitignore')).toBe(false);
    expect(await axeViolations()).toEqual([]);

    await browser.keys(Key.Escape);
    await browser.waitUntil(async () => !(await listShows()), { timeoutMsg: 'Escape did not close the list' });
    expect(await $(COMPOSER).getValue()).toBe('@');
    await clear();
  });

  it('goes into a folder and inserts files from the keyboard, with a chip for each mention', async () => {
    await type('Explain @do');
    await waitForOption('docs/');
    await browser.keys(Key.Enter);
    await waitForOption('docs/guide.md');
    await browser.keys(Key.Enter);
    await type('with @src/mai');
    await waitForOption('src/main.rs');
    await browser.keys(Key.Tab);

    expect(await $(COMPOSER).getValue()).toBe('Explain @docs/guide.md with @src/main.rs ');
    expect(await listShows()).toBe(false);
    expect(await chipLabels()).toEqual(['docs/guide.md', 'src/main.rs']);
  });

  it('sends the mentions, which the transcript shows and the model reads after the message', async () => {
    const before = await turnCount();
    await browser.keys(Key.Enter);
    await waitForNewTurn(before);
    await waitForCompleteReply();

    const kinds = (await latestTurnParts()).map((part) => part.kind);
    expect(kinds.slice(0, 3)).toEqual(['user_text', 'mention', 'mention']);
    const owner = latestOwnerMessage(provider);
    expect(owner).toContain('Explain @docs/guide.md with @src/main.rs');
    expect(owner).toContain('The owner mentioned the file docs/guide.md.');
    expect(owner).toContain('fn main() { println!("orbit"); }');
    expect(await chipLabels()).toEqual([]);
  });

  it('finds a name with a space as the owner types it, and inserts it in quotes', async () => {
    await type('Plan from @release pl');
    await waitForOption('docs/release plan.md');
    await browser.keys(Key.Enter);

    expect(await $(COMPOSER).getValue()).toBe('Plan from @"docs/release plan.md" ');
    expect(await listShows()).toBe(false);
    expect(await chipLabels()).toEqual(['docs/release plan.md']);
    await clear();
  });

  it('closes the list with Escape before Escape stops a reply', async () => {
    await useProfile('Hang');
    const before = await turnCount();
    await type('Wait for me');
    await browser.keys(Key.Enter);
    await waitForNewTurn(before);
    await waitForLatestPart('text', ['streaming']);

    await type(' @');
    await waitForOption('src/');
    await browser.keys(Key.Escape);
    await browser.waitUntil(async () => !(await listShows()), { timeoutMsg: 'Escape did not close the list' });
    const stillWriting = (await latestTurnParts()).some((part) => part.state === 'streaming');
    await browser.keys(Key.Escape);
    await waitForLatestPart('text', ['stopped']);

    expect(stillWriting).toBe(true);
    await clear();
    await useProfile('Echo');
  });

  it('says that nothing can be mentioned in a general conversation', async () => {
    await startNewConversation();
    await type('@');
    await browser.waitUntil(listShows, { timeoutMsg: 'the list did not open' });

    await browser.waitUntil(
      async () => {
        const hasNoOptions = (await optionLabels()).length === 0;
        const shows = await listShows();
        return hasNoOptions && shows;
      },
      { timeoutMsg: 'the list of a general conversation showed options' },
    );
    expect(await $('[data-mention-list]').getText()).not.toBe('');
    expect(await axeViolations()).toEqual([]);
    await browser.keys(Key.Escape);
    await clear();
  });
});
