// Projects in the sidebar (docs/features/chat/projects.md, acceptance): "+"
// adds a folder through the folder dialog's stand-in, a project's
// conversations work in its folder, the group and each project collapse from
// the mouse and the keyboard, a general conversation moves into the project
// that its message names, grants last for a project or for every
// conversation, the rows keep their place while their buttons show on hover,
// and removing a project deletes its conversations after saying how many.
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { Key } from 'webdriverio';

import {
  addProfile,
  addProject,
  allowFor,
  callMessage,
  clickInLatestTurn,
  clickWithText,
  latestToolStatus,
  projectRow,
  projectToggle,
  send,
  sendLines,
  startConversationIn,
  startNewConversation,
  waitForCompleteReply,
  waitForPointerInput,
  waitForToolStatus,
} from '../support/chat.ts';
import { deleteProviderProfiles } from '../support/harness.ts';
import {
  axeViolations,
  leaveSettings,
  openSettings,
  waitForAnimations,
  waitForApp,
  waitForShownText,
} from '../support/page.ts';
import { MODELS, type ProviderServer, startProviderServer, TOOLS_DONE } from '../support/provider-server.ts';

const DIALOG = '[role="dialog"]';

const ALERT_DIALOG = '[role="alertdialog"]';

/** The name of the test's project: the name of its folder. */
const PROJECT_NAME = 'atlas';

/** Every character of a text as a JSON escape, so that a message carries the text without naming it. */
function escaped(text: string): string {
  let result = '';
  for (let index = 0; index < text.length; index += 1) {
    result += `\\u${text.charCodeAt(index).toString(16).padStart(4, '0')}`;
  }
  return result;
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

/** The IDs of the conversations that the sidebar shows under a project. */
async function conversationsOf(projectId: string): Promise<string[]> {
  return browser.execute(
    (id: string) =>
      [...document.querySelectorAll(`[data-project="${id}"] [data-conversation]`)].map(
        (row) => (row as HTMLElement).dataset.conversation ?? '',
      ),
    projectId,
  );
}

/** How many "Revoke" buttons the main area shows. */
async function revokeButtons(): Promise<number> {
  return browser.execute(
    () =>
      [...document.querySelectorAll('main button')].filter((button) => button.textContent.trim() === 'Revoke').length,
  );
}

/** The conversation that the window shows. */
async function openConversation(): Promise<string> {
  return browser.execute(() => new URLSearchParams(location.search).get('conversation') ?? '');
}

/** Waits until the window shows a conversation, after its first message. */
async function waitForOpenConversation(): Promise<string> {
  let id = '';
  await browser.waitUntil(
    async () => {
      id = await openConversation();
      return id !== '';
    },
    { timeout: 10_000, timeoutMsg: 'the conversation did not open' },
  );
  return id;
}

/** A row of the sidebar: the button that fills it and one of the buttons that it shows on hover. */
interface SidebarRow {
  readonly action: string;
  readonly button: string;
}

/** How a row looks at a moment. */
interface RowLook {
  readonly background: string;
  readonly button: string;
  /** The opacity that the hover button shows with, with its ancestors' opacity. */
  readonly opacity: number;
  readonly title: string;
}

/** Where a row's button and title lie, the row's background, and how much its hover buttons show. */
async function rowLook(row: SidebarRow): Promise<RowLook> {
  return browser.execute(
    (buttonSelector: string, actionSelector: string) => {
      const button = document.querySelector(buttonSelector);
      const title = button?.querySelector('[data-content]');
      const action = document.querySelector(actionSelector);
      const place = (element: Element | null | undefined): string => {
        const rect = element?.getBoundingClientRect();
        return rect === undefined ? '' : [rect.x, rect.y, rect.width, rect.height].join(',');
      };
      // A row may hide or fade the element around its buttons rather than the buttons.
      let opacity = action?.checkVisibility() === true ? 1 : 0;
      for (let element = action; element !== null; element = element.parentElement) {
        opacity *= Number(getComputedStyle(element).opacity);
      }
      return {
        background: button === null ? '' : getComputedStyle(button).backgroundColor,
        button: place(button),
        opacity,
        title: place(title),
      };
    },
    row.button,
    row.action,
  );
}

/** Opens a project's menu, which its row shows on hover, and chooses an item. */
async function chooseProjectAction(projectId: string, label: string): Promise<void> {
  await waitForPointerInput();
  await projectToggle(projectId).moveTo();
  await $(`[data-project="${projectId}"] [data-project-menu]`).click();
  await clickWithText('[data-slot="dropdown-menu-item"]', label);
}

describe('projects', () => {
  let provider: ProviderServer;
  let folders: string;
  let folder: string;
  let outside: string;
  let project: string;

  before(async () => {
    provider = await startProviderServer();
    // The folders as the system names them, which is how the core stores them.
    folders = realpathSync.native(mkdtempSync(join(tmpdir(), 'mannie-e2e-projects-')));
    folder = join(folders, PROJECT_NAME);
    outside = join(folders, 'outside');
    mkdirSync(folder);
    mkdirSync(outside);
    writeFileSync(join(folder, 'a.txt'), 'a line in atlas\n');
    await waitForApp();
    await addProfile(provider.baseUrl, { model: MODELS.tools, name: 'Tools', useTools: true });
  });

  after(async () => {
    await deleteProviderProfiles();
    await provider.close();
    rmSync(folders, { force: true, maxRetries: 20, recursive: true, retryDelay: 250 });
  });

  it('adds the folder chosen in the folder dialog as a project, which opens with a new conversation', async () => {
    project = await addProject(folder);

    await waitForShownText(projectRow(project), expect.stringContaining(PROJECT_NAME));
    await waitForShownText($('main'), expect.stringContaining(`Start a conversation in ${PROJECT_NAME}`));
    await axeInBothThemes();

    // The same folder again opens the same project.
    expect(await addProject(folder)).toBe(project);
    expect(await $$('[data-project]').length).toBe(1);
  });

  it('works in the project’s folder without asking', async () => {
    await send(callMessage('read_file', { path: 'a.txt' }));

    const reply = await waitForCompleteReply();
    expect(reply).toContain(TOOLS_DONE);
    expect(reply).toContain('a line in atlas');
    expect(await latestToolStatus()).toBe('completed');
    expect(await conversationsOf(project)).toEqual([await openConversation()]);
  });

  it('collapses and expands the group and the project from the mouse and the keyboard', async () => {
    await projectToggle(project).click();
    await expect(projectToggle(project)).toHaveAttribute('aria-expanded', 'false');
    await expect(projectRow(project).$('[data-conversation]')).not.toBeDisplayed();

    await projectToggle(project).click();
    await expect(projectToggle(project)).toHaveAttribute('aria-expanded', 'true');

    const group = $('[data-slot="sidebar-group-label"][aria-expanded]');
    await group.click();
    await expect(group).toHaveAttribute('aria-expanded', 'false');
    await expect(projectRow(project)).not.toBeDisplayed();
    await browser.keys(Key.Enter);
    await expect(group).toHaveAttribute('aria-expanded', 'true');
    await expect(projectRow(project)).toBeDisplayed();

    // The collapsed project stays collapsed after a reload.
    await projectToggle(project).click();
    await browser.refresh();
    await waitForApp();
    await expect(projectToggle(project)).toHaveAttribute('aria-expanded', 'false');
    await projectToggle(project).click();
  });

  it('shows five conversations of a project, and the rest after "Show more"', async () => {
    for (let index = 0; index < 5; index += 1) {
      await startConversationIn(project);
      await send(`Message ${String(index + 1)}`);
      await waitForCompleteReply();
    }

    // The project has six conversations, the newest first: five show.
    await startConversationIn(project);
    expect(await conversationsOf(project)).toHaveLength(5);
    await clickWithText(`[data-project="${project}"] button`, 'Show more');
    expect(await conversationsOf(project)).toHaveLength(6);
    await clickWithText(`[data-project="${project}"] button`, 'Show less');
    expect(await conversationsOf(project)).toHaveLength(5);
  });

  it('shows a reply that waits for approval on a collapsed project', async () => {
    await send(callMessage('write_file', { content: 'x', path: join(outside, 'waiting.txt') }));
    await waitForToolStatus('awaiting_approval');

    await projectToggle(project).click();
    await expect(projectRow(project).$('svg[aria-label="A conversation waits for your approval"]')).toBeDisplayed();
    await projectToggle(project).click();

    await clickInLatestTurn('Deny');
    await waitForCompleteReply();
    expect(existsSync(join(outside, 'waiting.txt'))).toBe(false);
  });

  it('moves a general conversation into the project that its message names, without asking', async () => {
    await startNewConversation();
    // A move into a collapsed project expands it.
    await projectToggle(project).click();
    await expect(projectToggle(project)).toHaveAttribute('aria-expanded', 'false');
    // One step: the move runs first, and the read after it works in the project's folder.
    await sendLines([
      callMessage('move_to_project', { project: PROJECT_NAME }),
      callMessage('read_file', { path: 'a.txt' }),
    ]);

    expect(await waitForCompleteReply()).toContain('a line in atlas');
    const conversation = await waitForOpenConversation();
    await expect(projectToggle(project)).toHaveAttribute('aria-expanded', 'true');
    expect(await conversationsOf(project)).toContain(conversation);
    // The move is the turn's first call.
    const moveRow = (await $$('main article:last-of-type [data-tool-toggle]'))[0];
    await moveRow?.click();
    await waitForShownText($('main article:last-of-type'), expect.stringContaining(`Moved to ${PROJECT_NAME}`));
  });

  it('asks before a move that the message does not name', async () => {
    await startNewConversation();
    // The fixture's model reads the escaped name; the owner's message never names the project.
    await send(`call move_to_project {"project":"${escaped(PROJECT_NAME)}"}`);
    await waitForToolStatus('awaiting_approval');

    await waitForShownText(
      $('main article:last-of-type'),
      expect.stringContaining(`Move this conversation to the project ${PROJECT_NAME}`),
    );
    await expect($('main article:last-of-type [data-allow-for]')).not.toBeExisting();
    await axeInBothThemes();
    await clickInLatestTurn('Allow once');

    await waitForCompleteReply();
    expect(await conversationsOf(project)).toContain(await waitForOpenConversation());
  });

  it('allows a change for every conversation of the project, and for every conversation', async () => {
    await startConversationIn(project);
    await send(callMessage('write_file', { content: '1', path: join(outside, 'one.txt') }));
    await waitForToolStatus('awaiting_approval');
    await allowFor('This project');
    await waitForCompleteReply();

    await startConversationIn(project);
    await send(callMessage('write_file', { content: '2', path: join(outside, 'two.txt') }));
    expect(await waitForCompleteReply()).toContain(TOOLS_DONE);
    expect(await latestToolStatus()).toBe('completed');
    expect(readFileSync(join(outside, 'two.txt'), 'utf8')).toBe('2');

    await startNewConversation();
    await send(callMessage('write_file', { content: '3', path: join(outside, 'three.txt') }));
    await waitForToolStatus('awaiting_approval');
    await allowFor('Every conversation');
    await waitForCompleteReply();

    await startNewConversation();
    await send(callMessage('write_file', { content: '4', path: join(outside, 'four.txt') }));
    expect(await waitForCompleteReply()).toContain(TOOLS_DONE);
    expect(readFileSync(join(outside, 'four.txt'), 'utf8')).toBe('4');

    await openSettings('tools.settings');
    await waitForShownText($('main'), expect.stringContaining(`In the project ${PROJECT_NAME}`));
    await waitForShownText($('main'), expect.stringContaining('In every conversation'));
    for (const remaining of [1, 0]) {
      await clickWithText('main button', 'Revoke');
      await browser.waitUntil(async () => (await revokeButtons()) === remaining, {
        timeoutMsg: 'the grant stayed in the list',
      });
    }
    await waitForShownText($('main'), expect.stringContaining('No permissions'));
    await leaveSettings();
  });

  it('keeps each row in place while its buttons show on hover', async () => {
    const nested = (await conversationsOf(project))[0] ?? '';
    const general = await browser.execute(
      () =>
        [...document.querySelectorAll<HTMLElement>('[data-conversation]')].find(
          (row) => row.closest('[data-project]') === null,
        )?.dataset.conversation ?? '',
    );
    expect([nested, general]).not.toContain('');
    const rows: SidebarRow[] = [
      {
        action: `[data-project="${project}"] [data-project-menu]`,
        button: `[data-project="${project}"] [data-project-toggle]`,
      },
      ...[nested, general].map((id) => ({
        action: `[data-conversation="${id}"] [data-conversation-menu]`,
        button: `[data-conversation="${id}"] a`,
      })),
    ];
    // A focused row shows its buttons too.
    await browser.execute(() => {
      (document.activeElement as HTMLElement | null)?.blur();
    });
    await waitForPointerInput();

    for (const row of rows) {
      await $('main').moveTo();
      await browser.waitUntil(async () => (await rowLook(row)).opacity === 0, {
        timeoutMsg: `${row.button} shows its buttons without the pointer`,
      });
      const resting = await rowLook(row);

      await $(row.button).moveTo();
      await browser.waitUntil(async () => (await rowLook(row)).opacity === 1, {
        timeoutMsg: `${row.button} did not show its buttons on hover`,
      });
      const hovered = await rowLook(row);
      expect(hovered.button).toBe(resting.button);
      expect(hovered.title).toBe(resting.title);
      expect(hovered.background).not.toBe('rgba(0, 0, 0, 0)');

      // The row stays highlighted while the pointer is on one of its buttons.
      await $(row.action).moveTo();
      expect((await rowLook(row)).background).toBe(hovered.background);
    }
    await $('main').moveTo();
  });

  it('renames the project', async () => {
    await chooseProjectAction(project, 'Rename…');
    const dialog = $(DIALOG);
    await dialog.waitForDisplayed();
    await waitForAnimations();
    expect(await axeViolations()).toEqual([]);
    const field = $('#rename-project-name');
    await field.clearValue();
    await field.setValue('Atlas app');
    await clickWithText(`${DIALOG} button`, 'Save');
    await dialog.waitForDisplayed({ reverse: true });

    await waitForShownText(projectRow(project), expect.stringContaining('Atlas app'));
  });

  it('removes the project after saying how many conversations go, and deletes them', async () => {
    await clickWithText(`[data-project="${project}"] button`, 'Show more');
    const conversations = await conversationsOf(project);
    expect(conversations.length).toBeGreaterThan(5);

    await chooseProjectAction(project, 'Remove…');
    const dialog = $(ALERT_DIALOG);
    await dialog.waitForDisplayed();
    await waitForAnimations();
    await waitForShownText(dialog, expect.stringContaining(`its ${String(conversations.length)} conversations`));
    expect(await axeViolations()).toEqual([]);
    await clickWithText(`${ALERT_DIALOG} button`, `Remove and delete ${String(conversations.length)} conversations`);
    await dialog.waitForDisplayed({ reverse: true });

    await expect(projectRow(project)).not.toBeExisting();
    const listed = await browser.execute(() =>
      [...document.querySelectorAll('[data-conversation]')].map((row) => (row as HTMLElement).dataset.conversation),
    );
    for (const conversation of conversations) {
      expect(listed).not.toContain(conversation);
    }
    // The folder and its files stay.
    expect(readFileSync(join(folder, 'a.txt'), 'utf8')).toContain('a line in atlas');
  });
});
