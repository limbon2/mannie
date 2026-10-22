// Background commands in the conversation view (docs/features/system-tools/
// background-commands.md and wake-ups.md, acceptance): a command started in
// the background keeps running after its reply, the conversation's control
// "Commands" counts it and opens its output in a side panel that shows lines
// as they arrive, Stop in the panel ends it, the call's card shows its
// state, and a command that ends on its own after the reply wakes the agent,
// whose row names it. In the pseudo-locale, they show only catalog text
// beside the commands and their output. The fixture provider's tools
// scenario calls the tools that each message names.
import { mkdtempSync, realpathSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
  addProfile,
  addProject,
  callMessage,
  clickInLatestTurn,
  clickWithText,
  LATEST_MAIN_TURN,
  projectRow,
  send,
  waitForCompleteReply,
  waitForToolStatus,
} from '../support/chat.ts';
import { deleteProviderProfiles } from '../support/harness.ts';
import {
  axeViolations,
  leaveSettings,
  switchToPseudoLocale,
  uncataloguedText,
  waitForAnimations,
  waitForApp,
  waitForShownText,
} from '../support/page.ts';
import { MODELS, type ProviderServer, startProviderServer, TOOLS_DONE } from '../support/provider-server.ts';

const IS_WINDOWS = process.platform === 'win32';

/** A command that writes a numbered line every half second for a minute, in the platform's shell. */
const TICKING = IS_WINDOWS
  ? '1..120 | ForEach-Object { "tick $_"; Start-Sleep -Milliseconds 500 }'
  : 'i=1; while [ $i -le 120 ]; do echo "tick $i"; i=$((i+1)); sleep 0.5; done';

/** A command that ends on its own three seconds after it started. */
const ENDS_SOON = IS_WINDOWS ? 'Start-Sleep -Seconds 3; Write-Output built' : 'sleep 3; echo built';

/** The conversation's control "Commands". */
const CONTROL = '[data-background-commands]';

/** The output panel of a command. */
const PANEL = '[data-command-output]';

/** Starts `command` in the background and allows it once. */
async function startInBackground(command: string): Promise<void> {
  await send(callMessage('start_command', { activity: 'Starting in the background', command }));
  await waitForToolStatus('awaiting_approval');
  await waitForShownText($(LATEST_MAIN_TURN), expect.stringContaining('Runs in the background'));
  await clickInLatestTurn('Allow once');
  expect(await waitForCompleteReply()).toContain(TOOLS_DONE);
}

describe('background commands', () => {
  let provider: ProviderServer;
  let workspace: string;
  /** The project whose conversation runs the commands. */
  let project: string;

  before(async () => {
    provider = await startProviderServer();
    workspace = realpathSync.native(mkdtempSync(join(tmpdir(), 'mannie-e2e-background-')));
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

  it('keeps a command running after its reply, counts it in the toolbar and shows its output as it arrives', async () => {
    await startInBackground(TICKING);

    const control = await $(CONTROL);
    await control.waitForDisplayed();
    await waitForShownText(control, expect.stringContaining('1'));
    await control.click();
    await waitForAnimations();
    expect(await axeViolations()).toEqual([]);
    // The menu lists the running command first, with its number and its state.
    const item = await $('[role="menuitem"]');
    await waitForShownText(item, expect.stringContaining('Command 1:'));
    await item.click();

    const panel = await $(PANEL);
    await panel.waitForDisplayed();
    await waitForShownText($(`${PANEL} [role="log"]`), expect.stringContaining('tick 3'));
    await waitForShownText(panel, expect.stringContaining('Running for'));
    await waitForAnimations();
    expect(await axeViolations()).toEqual([]);
  });

  it('stops the command from its output panel, and the call’s card shows how it ended', async () => {
    await clickWithText(`${PANEL} button`, 'Stop');

    await waitForShownText($(PANEL), expect.stringContaining('Stopped by you'));
    await waitForShownText($(PANEL), expect.stringContaining('Show full output'));
    await $('[data-side-panel-close]').click();
    await waitForShownText($(LATEST_MAIN_TURN), expect.stringContaining('Stopped by you'));
    // The control shows no count once nothing runs.
    await expect($(`${CONTROL} [data-slot="badge"]`)).not.toBeExisting();
  });

  it('wakes the agent when a command ends on its own after the reply', async () => {
    await startInBackground(ENDS_SOON);

    const note = $('main [data-command-note]');
    await note.waitForDisplayed({ timeout: 30_000 });
    await waitForShownText(note, expect.stringContaining('Command 2 ended, and the agent continued'));
    await waitForShownText(note, expect.stringContaining('Ended with exit code 0'));
    expect(await waitForCompleteReply()).toContain(TOOLS_DONE);
    expect(await axeViolations()).toEqual([]);
  });

  it('shows only catalog text in the control, its menu, the output panel and the wake-up row in the pseudo-locale', async () => {
    // The selectors use structure only, because every catalog string is
    // pseudo-localized.
    await switchToPseudoLocale();
    await leaveSettings();
    await projectRow(project).$('[data-conversation] a').click();
    const control = await $(CONTROL);
    await control.waitForDisplayed();
    await $('main [data-command-note]').waitForDisplayed();
    await waitForAnimations();
    expect(await uncataloguedText()).toEqual([]);

    await control.click();
    await $('[data-slot="dropdown-menu-content"]').waitForDisplayed();
    await waitForAnimations();
    expect(await uncataloguedText()).toEqual([]);

    // Both commands ended; the first one listed opens its output.
    const [first] = await $$('[role="menuitem"]');
    await first?.click();
    await $(PANEL).waitForDisplayed();
    await waitForAnimations();
    expect(await uncataloguedText()).toEqual([]);
    expect(await axeViolations()).toEqual([]);
  });
});
