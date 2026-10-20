// Child agents in the conversation view (docs/features/child-agents/
// child-agent-activity.md, acceptance): a task's card shows its child's
// request with the task's number and title, the child's report reaches the
// parent once the owner allows the request, the child's conversation opens
// read-only in a side panel that works from the keyboard (ADR-0025), a limit
// of one child makes a second task wait for the slot, the owner stops one
// child while the parent continues, and a background task's report comes back
// through wait_for_tasks. The fixture provider's tools scenario calls the
// tools that each message names, and a task's instruction scripts its child
// the same way.
import { mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { Key } from 'webdriverio';

import {
  addProfile,
  addProject,
  callMessage,
  clickInLatestTurn,
  clickWithText,
  LATEST_MAIN_TURN,
  listedConversations,
  send,
  sendLines,
  startConversationIn,
  waitForCompleteReply,
  waitForPointerInput,
  waitForToolStatus,
} from '../support/chat.ts';
import { deleteProviderProfiles } from '../support/harness.ts';
import {
  axeViolations,
  focusedElement,
  leaveSettings,
  openSettings,
  runCommand,
  shownText,
  waitForApp,
  waitForShownText,
} from '../support/page.ts';
import { MODELS, type ProviderServer, startProviderServer, TOOLS_DONE } from '../support/provider-server.ts';

/** The side panel area. */
const PANELS = '[data-side-panels]';

/** The tab of the visible side panel. */
const SHOWN_TAB = `${PANELS} [role="tab"][aria-selected="true"]`;

/** The handle between the main view and the side panels. */
const RESIZE_HANDLE = '[data-slot="resizable-handle"]';

/** The child agents' button in the main view's conversation toolbar. */
const CHILD_AGENTS_BUTTON = 'main [data-child-agents]';

/** The items of an open menu. */
const MENU_ITEMS = '[data-slot="dropdown-menu-content"] [data-slot="dropdown-menu-item"]';

/** Where PaneForge keeps the sizes of the shell's panes: its prefix and the shell's `autoSaveId`. */
const SAVED_LAYOUT_KEY = 'paneforge:mannie.shell.side-panels';

/** A task for the tools scenario's `start_task`, in the foreground unless it says otherwise. */
interface TaskCall {
  readonly agent: 'explore' | 'general';
  readonly background?: boolean;
  /** The child's script, written as the owner's message scripts the parent. */
  readonly instruction: string;
  readonly title: string;
}

/** The panels that the address holds, and the visible one. */
interface PanelAddress {
  readonly panels: string[];
  readonly shown: string | null;
}

/** The line of a message that makes the parent start a task. */
function startTask(task: TaskCall): string {
  return callMessage('start_task', { background: false, ...task });
}

/** The card of the task with this number in the latest turn of the main view. */
function card(number: number): string {
  return `${LATEST_MAIN_TURN} [data-task-number="${String(number)}"]`;
}

/** Waits until the card of a task shows the task in `state`. */
async function waitForTaskState(number: number, state: string, timeout = 30_000): Promise<void> {
  await $(`${card(number)}[data-task-state="${state}"]`).waitForExist({
    timeout,
    timeoutMsg: `task ${String(number)} did not become ${state}`,
  });
}

/**
 * Clicks a button of a task's card: "Open conversation", Stop or Details. The
 * button moves to the middle of the transcript first, where the button that
 * jumps to the newest message never covers it.
 */
async function clickInCard(number: number, button: 'details' | 'open' | 'stop'): Promise<void> {
  const target = $(`${card(number)} [data-task-${button}]`);
  await target.scrollIntoView({ block: 'center' });
  await target.click();
}

/**
 * Opens the conversation of a task's child in a side panel, and waits until
 * its tab is the visible one and its turns show.
 */
async function openChild(number: number): Promise<void> {
  await clickInCard(number, 'open');
  await waitForShownText(SHOWN_TAB, new RegExp(`^Task ${String(number)}:`, 'u'));
  await $(`${PANELS} [role="tabpanel"]:not([hidden]) article`).waitForDisplayed({ timeout: 10_000 });
}

/** Reads the side panels from the window's address. */
async function panelAddress(): Promise<PanelAddress> {
  return browser.execute(() => {
    const search = new URLSearchParams(location.search);
    return { panels: search.getAll('panel'), shown: search.get('shown-panel') };
  });
}

/** The number of the task whose card holds the focus, or `null`. */
async function focusedTaskNumber(): Promise<string | null> {
  return browser.execute(
    () => document.activeElement?.closest<HTMLElement>('[data-task-number]')?.dataset.taskNumber ?? null,
  );
}

/** The labels of the side panels' tabs, in order. */
async function tabLabels(): Promise<string[]> {
  return browser.execute(
    (selector: string) => [...document.querySelectorAll(selector)].map((tab) => tab.textContent.trim()),
    `${PANELS} [role="tab"]`,
  );
}

/** Moves the focus backwards with Shift+Tab, as the owner would, until it reaches an element with `role`. */
async function tabBackTo(role: string): Promise<void> {
  for (let presses = 0; presses < 10; presses += 1) {
    const reached = await browser.execute(
      (wanted: string) => document.activeElement?.getAttribute('role') === wanted,
      role,
    );
    if (reached) {
      return;
    }
    await browser.keys([Key.Shift, Key.Tab]);
  }
  throw new Error(`Shift+Tab did not reach an element with the role ${role}`);
}

/** Waits until PaneForge has saved a layout of the panes, which it does a moment after a change. */
async function waitForSavedLayout(layout: string): Promise<void> {
  await browser.waitUntil(
    async () =>
      browser.execute(
        (key: string, saved: string) => (localStorage.getItem(key) ?? '').includes(saved),
        SAVED_LAYOUT_KEY,
        layout,
      ),
    { timeout: 5_000, timeoutMsg: `the layout ${layout} was not saved` },
  );
}

/** Whether the main view takes no input, as while a side panel covers it. */
async function mainViewIsInert(): Promise<boolean> {
  return browser.execute(() => document.querySelector('[data-pane-id="shell-main-pane"] > [inert]') !== null);
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

/** Chooses how many child agents work at once, on the settings page. */
async function chooseLimit(value: string, shown: string): Promise<void> {
  await openSettings('child-agents.settings');
  await $('#child-agents-limit').click();
  await $(`[data-slot="select-item"][data-value="${value}"]`).click();
  await waitForShownText('#child-agents-limit', shown);
  await waitForShownText($('main'), expect.stringContaining('Saved.'));
}

describe('child agents', () => {
  let provider: ProviderServer;
  let workspace: string;
  /** The project whose folder is the workspace of the parents and their children. */
  let project: string;

  before(async () => {
    provider = await startProviderServer();
    // The folder as the system names it, which is how the core stores it.
    workspace = realpathSync.native(mkdtempSync(join(tmpdir(), 'mannie-e2e-children-')));
    writeFileSync(join(workspace, 'notes.txt'), 'first line\nsecond line\n');
    await waitForApp();
    await addProfile(provider.baseUrl, { model: MODELS.tools, name: 'Tools', useTools: true });
    project = await addProject(workspace);
  });

  after(async () => {
    // The profiles' credentials live in the system's credential store.
    await deleteProviderProfiles();
    await provider.close();
    rmSync(workspace, { force: true, maxRetries: 20, recursive: true, retryDelay: 250 });
  });

  it('asks for a child’s change in its task’s card, and gives the parent the report once allowed', async () => {
    const instruction = callMessage('write_file', {
      activity: 'Writing the summary',
      content: 'made by a child',
      path: 'summary.md',
    });
    await send(startTask({ agent: 'general', instruction, title: 'Write the summary' }));
    await waitForToolStatus('awaiting_approval');

    // The card names the task that asks, so the request is not mistaken for the parent's.
    const shown = $(card(1));
    await waitForShownText(shown, expect.stringContaining('Task 1'));
    await waitForShownText(shown, expect.stringContaining('Write the summary'));
    await waitForShownText(shown, expect.stringContaining('General'));
    await waitForShownText(shown, expect.stringContaining('Waiting for your approval'));
    await waitForShownText(shown, expect.stringContaining('Writing the summary'));
    // The parent waits for the owner too, and the request leaves the focus in the message box.
    await waitForShownText($('main'), expect.stringContaining('Waiting for your approval.'));
    await expect($('[data-slot="sidebar"] svg[aria-label="Waiting for your approval"]')).toBeExisting();
    expect(await focusedElement()).toEqual(expect.objectContaining({ tag: 'textarea' }));
    await axeInBothThemes();

    await clickInLatestTurn('Allow once');

    expect(await waitForCompleteReply()).toContain(`${TOOLS_DONE} Last result: Task 1 reported:`);
    expect(readFileSync(join(workspace, 'summary.md'), 'utf8')).toBe('made by a child');
    await waitForTaskState(1, 'reported');
    await expect($('[data-slot="sidebar"] svg[aria-label="Waiting for your approval"]')).not.toBeExisting();
    // The sidebar lists the parent's conversation and none of its children's.
    expect(await listedConversations()).toHaveLength(1);
  });

  it('shows the instruction and the report on demand', async () => {
    await clickInCard(1, 'details');

    // The card's own details, not those of a call inside it.
    const details = $(`${card(1)} > [data-slot="collapsible"] > [data-slot="collapsible-content"]`);
    await waitForShownText(details, expect.stringContaining('summary.md'));
    await waitForShownText(details, expect.stringContaining(TOOLS_DONE));
  });

  it('opens the child’s conversation read-only in a side panel, and gives the focus back when it closes', async () => {
    await openChild(1);

    expect(await focusedElement()).toEqual({ label: 'Task 1: Write the summary', tag: 'h2' });
    await waitForShownText(SHOWN_TAB, 'Task 1: Write the summary');
    // The child's turn: its call and its report, with no message box.
    const panels = $(PANELS);
    await waitForShownText(panels, expect.stringContaining('Writing the summary'));
    await waitForShownText(panels, expect.stringContaining(TOOLS_DONE));
    await waitForShownText(panels, expect.stringContaining('This conversation belongs to a child agent.'));
    await expect($(`${PANELS} textarea`)).not.toBeExisting();
    // The main view stays beside the panel.
    await expect($(card(1))).toBeDisplayed();
    const address = await panelAddress();
    expect(address.panels).toEqual([expect.stringMatching(/^chat\.conversation:/u)]);
    expect(address.shown).toBe(address.panels[0]);
    await axeInBothThemes();

    await $('[data-side-panel-close]').click();

    await $(PANELS).waitForExist({ reverse: true });
    expect(await focusedElement()).toEqual({ label: 'Open conversation', tag: 'button' });
    expect(await focusedTaskNumber()).toBe('1');
    expect((await panelAddress()).panels).toEqual([]);
  });

  it('resizes the side panel from the keyboard, and keeps the panel and its size after a reload', async () => {
    await openChild(1);
    // The handle's value is the main view's share of the area, in percent.
    const handle = $(RESIZE_HANDLE);
    await expect(handle).toHaveAttribute('aria-valuenow', '60');
    await expect(handle).toHaveAttribute('aria-valuemin', '35');
    await expect(handle).toHaveAttribute('aria-valuemax', '75');

    await tabBackTo('separator');
    await browser.keys(Key.ArrowLeft);
    await expect(handle).toHaveAttribute('aria-valuenow', '50');
    await browser.keys(Key.End);
    await expect(handle).toHaveAttribute('aria-valuenow', '75');
    await browser.keys(Key.Home);
    await expect(handle).toHaveAttribute('aria-valuenow', '35');
    await waitForSavedLayout('[35,65]');

    await browser.refresh();
    await waitForApp();

    await $(`${PANELS} article`).waitForDisplayed({ timeout: 10_000 });
    await waitForShownText(SHOWN_TAB, 'Task 1: Write the summary');
    await expect($(RESIZE_HANDLE)).toHaveAttribute('aria-valuenow', '35');
    // A reload moves no focus.
    expect(await focusedElement()).not.toEqual(expect.objectContaining({ tag: 'h2' }));

    // The panel that opened with the window has no control to go back to.
    await $('[data-side-panel-close]').click();

    await $(PANELS).waitForExist({ reverse: true });
    expect(await focusedElement()).toEqual(expect.objectContaining({ tag: 'main' }));
  });

  it('covers the main view with the side panel in a narrow window, until the panel closes', async () => {
    const { height, width } = await browser.getWindowSize();
    await browser.setWindowSize(800, height);
    try {
      await openChild(1);

      // Without room beside the view, the panel covers it, and the view takes no input.
      await expect($(RESIZE_HANDLE)).not.toBeExisting();
      expect(await mainViewIsInert()).toBe(true);
      expect(await focusedElement()).toEqual({ label: 'Task 1: Write the summary', tag: 'h2' });

      await $('[data-side-panel-close]').click();

      await $(PANELS).waitForExist({ reverse: true });
      expect(await mainViewIsInert()).toBe(false);
      expect(await focusedTaskNumber()).toBe('1');
    } finally {
      await browser.setWindowSize(width, height);
    }
  });

  it('makes a second task wait for the only slot, and continues the parent when the owner stops each child', async () => {
    await chooseLimit('1', '1');
    await axeInBothThemes();
    await leaveSettings();
    await startConversationIn(project);

    await sendLines([
      startTask({ agent: 'explore', instruction: 'hang', title: 'Watch the first' }),
      startTask({ agent: 'explore', instruction: 'hang', title: 'Watch the second' }),
    ]);

    await waitForTaskState(1, 'running');
    await waitForTaskState(2, 'accepted');
    await waitForShownText($(card(2)), expect.stringContaining('Waiting for a free slot'));
    await waitForShownText($('main'), expect.stringContaining('Waiting for 2 child agents…'));
    // The toolbar counts the child agents that work, also the one that waits for its slot.
    await expect($(CHILD_AGENTS_BUTTON)).toHaveAttribute('aria-label', 'Child agents, 2 working');

    await clickInCard(1, 'stop');

    await waitForTaskState(1, 'canceled');
    await waitForShownText($(card(1)), expect.stringContaining('Stopped'));
    // The stopped child gave its slot to the waiting one.
    await waitForTaskState(2, 'running');
    await clickInCard(2, 'stop');
    await waitForTaskState(2, 'canceled');

    expect(await waitForCompleteReply()).toContain('Task 2 was stopped before it finished.');
  });

  it('shows a tab for each open child, keeps the hidden ones open, and closes them one at a time', async () => {
    await openChild(1);
    await openChild(2);

    expect(await tabLabels()).toEqual(['Task 1: Watch the first', 'Task 2: Watch the second']);
    await waitForShownText(SHOWN_TAB, 'Task 2: Watch the second');
    const [first, second] = (await panelAddress()).panels;
    expect((await panelAddress()).shown).toBe(second);

    // The tab list works with the arrow keys, and the hidden panel stays open.
    await tabBackTo('tab');
    await browser.keys(Key.ArrowLeft);
    await waitForShownText(SHOWN_TAB, 'Task 1: Watch the first');
    await browser.waitUntil(async () => (await panelAddress()).shown === first, {
      timeoutMsg: 'the address did not name the first panel',
    });
    await expect($$(`${PANELS} [role="tabpanel"]`)).toBeElementsArrayOfSize(2);
    await axeInBothThemes();

    // Delete closes the focused tab, and the focus moves to the tab that shows next.
    await browser.keys(Key.Delete);

    await waitForShownText(SHOWN_TAB, 'Task 2: Watch the second');
    expect(await tabLabels()).toEqual(['Task 2: Watch the second']);
    expect(await focusedElement()).toEqual({ label: 'Task 2: Watch the second', tag: 'button' });

    // The Close inside a tab closes its panel, also one that does not show.
    await openChild(1);
    await $(`[data-side-panel-tab="${String(second)}"] [data-side-panel-close]`).click();

    await browser.waitUntil(async () => (await tabLabels()).length === 1, { timeoutMsg: 'the tab did not close' });
    expect(await tabLabels()).toEqual(['Task 1: Watch the first']);
    expect(await focusedElement()).toEqual({ label: 'Task 1: Watch the first', tag: 'button' });
    await runCommand('Close the side panel');
    await $(PANELS).waitForExist({ reverse: true });
  });

  it('lists the conversation’s child agents in its toolbar, and opens one from there', async () => {
    const button = $(CHILD_AGENTS_BUTTON);
    // Both children were stopped, so none works now.
    await expect(button).toHaveAttribute('aria-label', 'Child agents');
    await waitForPointerInput();
    await button.click();
    await $('[data-slot="dropdown-menu-content"]').waitForDisplayed();

    const items = await $$(MENU_ITEMS).map(async (item) => shownText(item));
    expect(items).toHaveLength(2);
    expect(items[0]).toContain('Task 1: Watch the first');
    expect(items[0]).toContain('Stopped');
    expect(items[1]).toContain('Task 2: Watch the second');
    await axeInBothThemes();

    await clickWithText(`${MENU_ITEMS} [data-content]`, 'Task 2: Watch the second');

    await waitForShownText(SHOWN_TAB, 'Task 2: Watch the second');
    await $(`${PANELS} [role="tabpanel"]:not([hidden]) article`).waitForDisplayed({ timeout: 10_000 });
    expect(await focusedElement()).toEqual({ label: 'Task 2: Watch the second', tag: 'h2' });

    // Closing the panel gives the focus back to the toolbar's button.
    await $('[data-side-panel-close]').click();

    await $(PANELS).waitForExist({ reverse: true });
    expect(await focusedElement()).toEqual({ label: 'Child agents', tag: 'button' });
  });

  it('keeps the limit of child agents after a reload, and goes back to the default', async () => {
    await browser.refresh();
    await waitForApp();

    await openSettings('child-agents.settings');
    await waitForShownText('#child-agents-limit', '1');
    await chooseLimit('4', '4 (default)');
    await leaveSettings();
  });

  it('continues while a background task works, and reads its report with wait_for_tasks', async () => {
    await startConversationIn(project);
    const instruction = callMessage('read_file', { path: 'notes.txt' });

    await sendLines([
      startTask({ agent: 'explore', background: true, instruction, title: 'Read the notes' }),
      'next',
      callMessage('wait_for_tasks', { tasks: [1] }),
    ]);

    const reply = await waitForCompleteReply();
    expect(reply).toContain(`${TOOLS_DONE} Last result: Task 1 reported:`);
    expect(reply).toContain('first line');
    await waitForTaskState(1, 'reported');

    const wait = $(`${LATEST_MAIN_TURN} [data-task-wait]`);
    await waitForShownText(wait, expect.stringContaining('Read the reports'));
    await waitForShownText(wait, expect.stringContaining('Read the notes'));
    await waitForShownText(wait, expect.stringContaining('Reported'));
  });
});
