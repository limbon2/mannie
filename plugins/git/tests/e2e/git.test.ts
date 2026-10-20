// Git in the app (plugins/git/docs/README.md, acceptance), with a
// repository and a remote in temporary folders, run by the installed git:
// the Plugins page lists the installed program that the plugin runs; a read
// runs without asking and its card shows what it found; a commit that runs
// a hook asks first and names the hook, and one without a hook runs without
// asking once changes in the workspace do; a push names its endpoint, and a
// rule saved from it lets the next push run without asking; a commit's card
// opens to its patch; and the Git button in the toolbar opens the Git side
// panel. Every step passes axe in both themes. The fixture provider's tools
// scenario calls the tool that a message names. The plugin is off in a new
// installation, so the spec turns it on first.
import { execFileSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
  addProfile,
  addProject,
  callMessage,
  clickInLatestTurn,
  clickWithText,
  LATEST_MAIN_TURN,
  latestToolStatus,
  send,
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
  waitForAnimations,
  waitForApp,
} from '#e2e/page.ts';
import { MODELS, type ProviderServer, startProviderServer, TOOLS_DONE } from '#e2e/provider-server.ts';

/** The git plugin's row on the Plugins page. */
const PLUGIN = '[data-plugin="git"]';

const DIALOG = '[data-slot="dialog-content"]';

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

/** Runs the installed git in `folder`, as the owner would in a terminal, and returns its output. */
function git(folder: string, ...args: string[]): string {
  return execFileSync('git', args, { cwd: folder, encoding: 'utf8' });
}

/**
 * Removes the spec's folder. The app may still read the repository, as the
 * Git panel does when it shows, and Windows refuses to remove a folder that
 * a running git works in, so a refusal waits for the read to end.
 */
async function removeFolder(path: string): Promise<void> {
  await browser.waitUntil(
    () => {
      try {
        rmSync(path, { force: true, recursive: true });
        return true;
      } catch {
        return false;
      }
    },
    { interval: 500, timeout: 30_000, timeoutMsg: `the folder ${path} could not be removed` },
  );
}

/** Opens the latest call's details, where its card shows what the plugin found. */
async function openLatestCall(): Promise<void> {
  await $(`${LATEST_MAIN_TURN} [data-tool-toggle]`).click();
  await waitForAnimations();
}

/**
 * Checks that the approval of the latest turn lists, among what its run does
 * beyond the workspace, each of `expected`. The call's closed details keep a
 * hidden copy of the list, so the shown one is the approval's.
 */
async function expectShownEffects(expected: readonly string[]): Promise<void> {
  const shown = await $$(`${LATEST_MAIN_TURN} [data-program-effects]`).filter((list) => list.isDisplayed());
  expect(shown).toHaveLength(1);
  for (const list of shown) {
    for (const text of expected) {
      await expect(list).toHaveText(expect.stringContaining(text));
    }
  }
}

describe('Git', () => {
  let provider: ProviderServer;
  let folder: string;
  let workspace: string;
  let remote: string;
  /** The file that the repository's pre-commit hook writes to when it runs. */
  let hookMarker: string;

  before(async () => {
    provider = await startProviderServer();
    // The folders as the system names them, which is how the core stores them.
    folder = realpathSync.native(mkdtempSync(join(tmpdir(), 'mannie-e2e-git-')));
    workspace = join(folder, 'app');
    remote = join(folder, 'remote.git');
    hookMarker = join(folder, 'hook-ran.txt');
    git(folder, 'init', '--quiet', '--initial-branch=main', workspace);
    git(folder, 'init', '--quiet', '--bare', '--initial-branch=main', remote);
    // The repository's own settings win over the owner's global ones, so the
    // spec's commits need no identity or key of the owner's.
    const settings: readonly (readonly [string, string])[] = [
      ['user.name', 'E2E Author'],
      ['user.email', 'e2e@example.com'],
      ['commit.gpgSign', 'false'],
      ['core.hooksPath', '.git/hooks'],
    ];
    for (const [key, value] of settings) {
      git(workspace, 'config', key, value);
    }
    git(workspace, 'remote', 'add', 'origin', remote);
    writeFileSync(join(workspace, 'notes.txt'), 'first line\n');
    git(workspace, 'add', 'notes.txt');
    git(workspace, 'commit', '--quiet', '-m', 'Add the notes');
    // git runs hooks through its own shell, also on Windows.
    const marker = hookMarker.replaceAll('\\', '/');
    writeFileSync(join(workspace, '.git', 'hooks', 'pre-commit'), `#!/bin/sh\necho ran >> '${marker}'\n`, {
      mode: 0o755,
    });
    writeFileSync(join(workspace, 'notes.txt'), 'first line\nsecond line\n');
    writeFileSync(join(workspace, 'todo.txt'), 'write the tests\n');
    await waitForApp();
  });

  after(async () => {
    // The profiles' credentials live in the system's credential store.
    await deleteProviderProfiles();
    await provider.close();
    await removeFolder(folder);
  });

  it('lists Git off in a new installation, with the installed program that it runs', async () => {
    await openSettings('plugins.settings');
    await expect($(PLUGIN)).toHaveAttribute('data-plugin-state', 'off');

    await openPluginUses('git');

    await expect($(`${PLUGIN} [data-plugin-service="processes"] [data-plugin-service-sensitive]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-installed-program="git"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-command="repository_overview"]`)).toBeDisplayed();
    await expect($(`${PLUGIN} [data-plugin-channel="follow_repository"]`)).toBeDisplayed();
    await setPluginEnabled('git', true);
    await leaveSettings();
  });

  it('reads the status without asking, and its card lists the changed files', async () => {
    await addProfile(provider.baseUrl, { model: MODELS.tools, name: 'Tools', useTools: true });
    await addProject(workspace);

    await send(callMessage('git_status', {}));

    expect(await waitForCompleteReply()).toContain(TOOLS_DONE);
    expect(await latestToolStatus()).toBe('completed');
    await openLatestCall();
    const entries = $(`${LATEST_MAIN_TURN} [data-git-entries]`);
    await expect(entries).toHaveText(expect.stringContaining('modified: notes.txt'));
    await expect(entries).toHaveText(expect.stringContaining('todo.txt'));
    await axeInBothThemes();
  });

  it('asks before a commit that runs a hook, names the hook, and commits once allowed', async () => {
    await send(callMessage('git_commit', { all: true, message: 'Add the second line' }));
    await waitForToolStatus('awaiting_approval');

    await expectShownEffects(['Runs pre-commit from .git/hooks/pre-commit']);
    await axeInBothThemes();
    await clickInLatestTurn('Allow once');

    expect(await waitForCompleteReply()).toContain(TOOLS_DONE);
    expect(readFileSync(hookMarker, 'utf8').trim()).toBe('ran');
    expect(git(workspace, 'log', '-1', '--format=%s').trim()).toBe('Add the second line');
  });

  it('commits without asking once changes in the workspace run without asking and no hook runs', async () => {
    rmSync(join(workspace, '.git', 'hooks', 'pre-commit'));
    await openSettings('tools.settings');
    await $('#tools-allow-changes').click();
    await expect($('#tools-allow-changes')).toHaveAttribute('aria-checked', 'true');
    await leaveSettings();

    await send(callMessage('git_commit', { allow_empty: true, message: 'Commit without asking' }));

    expect(await waitForCompleteReply()).toContain(TOOLS_DONE);
    expect(await latestToolStatus()).toBe('completed');
    expect(git(workspace, 'log', '-1', '--format=%s').trim()).toBe('Commit without asking');
  });

  it('names the endpoint of a push, and a rule saved from it lets the next push run without asking', async () => {
    // Changes in the workspace run without asking, so a push asks for the
    // remote only, which the rule then covers.
    await send(callMessage('git_push', { set_upstream: true }));
    await waitForToolStatus('awaiting_approval');
    await expectShownEffects(['Connects to', 'remote.git']);

    await clickInLatestTurn('Save rule…');
    const dialog = await $(DIALOG);
    await dialog.waitForDisplayed();
    await expect($('[data-rule-program]')).toBeDisplayed();
    await axeInBothThemes();
    // The rule keeps the program's name and its subcommand: `git push`.
    await $('#tools-rule-words-2').click();
    await clickWithText(`${DIALOG} button`, 'Save rule and allow');
    await dialog.waitForDisplayed({ reverse: true });
    expect(await waitForCompleteReply()).toContain(TOOLS_DONE);
    expect(git(remote, 'log', '-1', '--format=%s', 'main').trim()).toBe('Commit without asking');

    git(workspace, 'commit', '--quiet', '--allow-empty', '-m', 'Push me without asking');
    await send(callMessage('git_push', {}));

    expect(await waitForCompleteReply()).toContain(TOOLS_DONE);
    expect(await latestToolStatus()).toBe('completed');
    expect(git(remote, 'log', '-1', '--format=%s', 'main').trim()).toBe('Push me without asking');
  });

  it('opens the card of a shown commit to its patch', async () => {
    const revision = git(workspace, 'log', '-1', '--format=%H', '--grep=Add the second line').trim();
    await send(callMessage('git_show', { revision }));
    expect(await waitForCompleteReply()).toContain(TOOLS_DONE);

    await openLatestCall();

    await expect($(`${LATEST_MAIN_TURN} [data-git-diff]`)).toHaveText(expect.stringContaining('notes.txt'));
    await expect($(`${LATEST_MAIN_TURN} [data-git-hunk]`)).toHaveText(expect.stringContaining('second line'));
    await axeInBothThemes();
  });

  it('opens the Git panel from the toolbar, with the branch, the changes and the history', async () => {
    const control = $('[data-git-control]');
    await expect(control).toHaveText(expect.stringContaining('main'));
    await control.click();

    const panel = $('[data-git-panel]');
    await expect(panel.$('[data-git-branch]')).toHaveText('On main');
    await expect(panel.$('[data-git-entries]')).toHaveText(expect.stringContaining('todo.txt'));
    await axeInBothThemes();
    await clickWithText('[data-git-panel] [role="tab"]', 'History');

    await expect(panel.$('[data-git-commits]')).toHaveText(expect.stringContaining('Push me without asking'));
    await axeInBothThemes();
  });

  it('lists the saved rule in the settings, where the owner deletes it', async () => {
    await openSettings('tools.settings');
    const rule = $('[data-program-rule]');
    await expect(rule).toHaveText(expect.stringContaining('git push'));

    await clickWithText('[data-program-rule] button', 'Delete');

    await expect(rule).not.toBeExisting();
    expect(existsSync(hookMarker)).toBe(true);
    await leaveSettings();
  });
});
