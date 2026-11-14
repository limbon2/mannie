// Terminals in the app (plugins/terminal/docs/README.md, acceptance): the
// Plugins page lists what the plugin may use; Ctrl+` opens a terminal of this
// computer beside a new conversation before its first message (ADR-0068),
// where the owner's command becomes a block of the command list; a command of the agent asks the owner with the
// terminal access and runs once allowed; the reply's citation shows the cited
// lines and opens them in the terminal; each terminal keeps its view while
// another tab shows, and follows its terminal again once it shows; a command
// goes into the message box as a mention; and the owner ends the terminal.
// Every step passes axe in both themes. The fixture provider's tools
// scenario calls the tool that a message names, and its answer repeats the
// tool's result with its cite link.
// The plugin is off in a new installation, so the spec turns it on first.
import {
  addProfile,
  callMessage,
  clickInLatestTurn,
  clickWithText,
  LATEST_MAIN_TURN,
  send,
  sendMessage,
  startNewConversation,
  waitForCompleteReply,
  waitForToolStatus,
} from '#e2e/chat.ts';
import { deleteProviderProfiles } from '#e2e/harness.ts';
import {
  axeViolations,
  leaveSettings,
  openPluginUses,
  openSettings,
  setPluginEnabled,
  shownText,
  waitForApp,
  waitForShownText,
} from '#e2e/page.ts';
import { MODELS, type ProviderServer, startProviderServer } from '#e2e/provider-server.ts';
import { Key } from 'webdriverio';

/** The plugin's row on the Plugins page. */
const PLUGIN = '[data-plugin="terminal"]';

/** The terminal's area in its side panel, which xterm.js fills. */
const VIEW = '[data-terminal-view]';

/** The menus of the command list's rows. */
const MENU_ITEM = '[data-slot="dropdown-menu-item"]';

declare global {
  interface Window {
    /** The xterm.js elements of the open terminals, in the order of their tabs. */
    keptTerminals?: Element[];
  }
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

/** Types a line into the terminal that has the focus, and Enter. */
async function typeLine(text: string): Promise<void> {
  // browser.keys presses the whole sequence before releasing any key.
  // Release each key so WebKitWebDriver receives repeated spaces and letters.
  const typing = browser.action('key');
  for (const character of text) {
    const key = character === ' ' ? Key.Space : character;
    typing.down(key).up(key);
  }
  await typing.down(Key.Enter).up(Key.Enter).perform();
}

/** What the badge of a block that runs says. */
const RUNNING = 'Running';

/** The command list's row of the block whose command is `command`, once it ended. */
async function endedBlock(command: string): Promise<WebdriverIO.Element> {
  let found: WebdriverIO.Element | undefined;
  let listed: string[] = [];
  try {
    await browser.waitUntil(
      async () => {
        listed = [];
        for (const row of await $$('[data-side-panels] [role="tabpanel"]:not([hidden]) [data-block]')) {
          const text = await shownText(row);
          listed.push(text.replaceAll('\n', ' '));
          if (text.includes(command) && !text.includes(RUNNING)) {
            found = row;
            return true;
          }
        }
        return false;
      },
      { timeout: 30_000 },
    );
  } catch (cause) {
    throw new Error(`no ended block of "${command}"; the list shows: ${JSON.stringify(listed)}`, { cause });
  }
  if (found === undefined) {
    throw new Error(`no ended block of "${command}"; the list shows: ${JSON.stringify(listed)}`);
  }
  return found;
}

/**
 * Waits for the shown terminal to reach a prompt before typing into it.
 */
async function waitForPrompt(): Promise<void> {
  await $('[data-side-panels] [role="tabpanel"]:not([hidden]) [data-terminal-phase="prompt"]').waitForDisplayed({
    timeout: 30_000,
    timeoutMsg: 'the terminal reached no prompt',
  });
}

/** Shows the command list of the panel that shows, unless it shows already. */
async function showCommandList(): Promise<void> {
  const pressed = await browser.execute(() => {
    const toggles = [...document.querySelectorAll<HTMLButtonElement>('[role="toolbar"] button[aria-pressed]')];
    const shown = toggles.find((toggle) => {
      const isShown = toggle.offsetParent !== null;
      const isCommands = toggle.textContent.trim() === 'Commands';
      return isShown && isCommands;
    });
    return shown?.getAttribute('aria-pressed') ?? null;
  });
  if (pressed !== 'true') {
    await clickWithText('[role="toolbar"] button', 'Commands');
  }
}

/** Moves to a neighboring terminal tab from the keyboard and waits for it to show. */
async function showTab(terminal: string, direction: typeof Key.ArrowLeft | typeof Key.ArrowRight): Promise<void> {
  const selected = '[data-side-panel-tab] [role="tab"][aria-selected="true"]';
  await $(selected).waitForDisplayed();
  await browser.execute((selector: string) => document.querySelector<HTMLElement>(selector)?.focus(), selected);
  await browser.keys(direction);
  await waitForShownText(selected, expect.stringContaining(`${terminal} ·`));
}

/** Whether the terminals' xterm.js elements are the ones that the test noted, in the same order. */
async function keepsTerminals(): Promise<boolean> {
  return browser.execute(() => {
    const kept = window.keptTerminals ?? [];
    const current = [...document.querySelectorAll('[data-side-panels] [data-terminal-view] .xterm')];
    const isSameCount = kept.length > 0 && current.length === kept.length;
    return isSameCount && current.every((element, index) => element === kept[index]);
  });
}

/** Chooses an action of a block's menu in the command list, whose button is the row's last. */
async function blockAction(row: WebdriverIO.Element, label: string): Promise<void> {
  const buttons = await row.$$('button');
  const menu = buttons.at(-1);
  if (menu === undefined) {
    throw new Error('a block has no menu');
  }
  await menu.click();
  await clickWithText(MENU_ITEM, label);
}

describe('Terminal', () => {
  let provider: ProviderServer;

  before(async () => {
    provider = await startProviderServer();
    await waitForApp();
  });

  after(async () => {
    // The profiles' credentials live in the system's credential store.
    await deleteProviderProfiles();
    await provider.close();
  });

  it('lists Terminal off in a new installation, with the service terminals, its crate and its commands', async () => {
    await openSettings('plugins.settings');
    await expect($(PLUGIN)).toHaveAttribute('data-plugin-state', 'off');

    await openPluginUses('terminal');

    await expect($(`${PLUGIN} [data-plugin-service="terminals"] [data-plugin-service-sensitive]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-crate="vt100"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-command="write_terminal"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-channel="follow_terminal"]`)).toBeDisplayed();
    await setPluginEnabled('terminal', true);
    await leaveSettings();
  });

  it("opens a terminal with Ctrl+` before the first message and turns the owner's command into a block", async () => {
    await addProfile(provider.baseUrl, { model: MODELS.tools, name: 'Tools', useTools: true });
    await startNewConversation();
    // A new conversation is the draft of its place, which the toolbar's
    // Terminal button and Ctrl+` work in before its first message (ADR-0068).
    await expect($('[data-terminal-control]')).toBeDisplayed();

    await browser.keys([Key.Ctrl, '`']);
    await $(`${VIEW} .xterm`).waitForDisplayed({ timeout: 30_000 });
    await waitForPrompt();
    await typeLine('echo hello from the owner');
    await showCommandList();
    const row = await endedBlock('echo hello from the owner');
    await blockAction(row, 'Show output as text');

    await waitForShownText($('[role="dialog"]'), expect.stringContaining('hello from the owner'));
    await axeInBothThemes();
    await browser.keys(Key.Escape);
    await axeInBothThemes();

    // The first message makes the draft a conversation of the list, and its
    // terminal stays beside it.
    await sendMessage('Open a conversation');
    await waitForCompleteReply();
    await expect($(`${VIEW} .xterm`)).toBeDisplayed();
  });

  it('runs an agent’s command once the owner allows it, and cites its output', async () => {
    await send(
      callMessage('run_in_terminal', {
        command: 'echo hello from the agent',
        host: null,
        shell: null,
        terminal: null,
        timeout_seconds: null,
      }),
    );
    await waitForToolStatus('awaiting_approval');

    // The call's closed details keep a hidden copy of the list, so the shown
    // one is the approval's.
    const shown = await $$(`${LATEST_MAIN_TURN} [data-terminal-command]`).filter((list) => list.isDisplayed());
    expect(shown).toHaveLength(1);
    await axeInBothThemes();
    await clickInLatestTurn('Allow once');

    expect(await waitForCompleteReply()).toContain('hello from the agent');
    await waitForToolStatus('completed');
    // The answer repeats the result, whose first line and output header both
    // cite the command.
    const [citation] = await $$(`${LATEST_MAIN_TURN} [data-citation]`).filter((shown) => shown.isDisplayed());
    expect(citation).toBeDefined();
    await citation?.click();
    await waitForShownText($('[data-slot="popover-content"]'), expect.stringContaining('hello from the agent'));
    await axeInBothThemes();
    await clickWithText('[data-slot="popover-content"] button', 'Show in terminal');

    // The agent's terminal shows in a panel of its own, beside the owner's.
    // Each poll reads the selected tab again: the navigation can land after
    // a first read, and an element found once keeps pointing at the old tab.
    await browser.waitUntil(
      async () =>
        browser.execute(() => {
          const selected = document.querySelector('[data-side-panel-tab] [role="tab"][aria-selected="true"]');
          return selected?.textContent.includes('Terminal 2') ?? false;
        }),
      { timeout: 10_000, timeoutMsg: "the agent's terminal did not become the panel that shows" },
    );
    await browser.waitUntil(async () => (await $$(VIEW).filter((view) => view.isDisplayed())).length === 1, {
      timeout: 10_000,
      timeoutMsg: "the agent's terminal did not show",
    });
  });

  it('keeps each terminal’s view while another tab shows, and follows its terminal again once it shows', async () => {
    // Terminal 2 shows, and terminal 1 waits behind its tab.
    await browser.execute(() => {
      window.keptTerminals = [...document.querySelectorAll('[data-side-panels] [data-terminal-view] .xterm')];
    });

    await showTab('Terminal 1', Key.ArrowLeft);
    const keptWhileTerminalOneShows = await keepsTerminals();
    await showTab('Terminal 2', Key.ArrowRight);
    await waitForPrompt();
    const [shownView] = await $$(VIEW).filter((view) => view.isDisplayed());
    await shownView?.click();
    await typeLine('echo after the switch');
    await showCommandList();
    await endedBlock('echo after the switch');
    await axeInBothThemes();

    expect(keptWhileTerminalOneShows).toBe(true);
    expect(await keepsTerminals()).toBe(true);
  });

  it('puts a command into the message box as a mention, and ends the terminal', async () => {
    await showCommandList();
    const row = await endedBlock('echo hello from the agent');
    await blockAction(row, 'Mention in message');

    await expect($('[data-mention-chips]')).toBeDisplayed();
    await axeInBothThemes();

    await clickWithText('[role="toolbar"] button', 'End');
    await $('button=Start again').waitForDisplayed({ timeout: 30_000 });
    await axeInBothThemes();
  });
});
