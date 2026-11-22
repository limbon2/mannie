// The Git side panel's states (plugins/git/docs/panel.md; ADR-0078),
// rendered on the server side with the repository's state given: loading,
// a failure with "Try again", a folder that is no repository, an unborn
// repository, a repository with its branch, upstream, operation banner,
// stash, changed files, confirmation dialog, and a refresh failure above
// the tabs.
import { render } from 'svelte/server';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import GitPanel from '../../../ui/components/GitPanel.svelte';
import type { RepositoryOverview } from '../../../ui/generated/bindings.js';
import type { ActionNotice, PendingConfirmation } from '../../../ui/state/git-repository.svelte.js';

/** The state of a value that the panel reads, as the test gives it. */
interface Given {
  failure: unknown;
  status: string;
  value: unknown;
}

const given = vi.hoisted(
  (): {
    actionNotice: ActionNotice | null;
    overview: Given;
    pendingConfirmation: PendingConfirmation | null;
  } => ({
    actionNotice: null,
    overview: { failure: null, status: 'idle', value: null },
    pendingConfirmation: null,
  }),
);

vi.mock('../../../ui/state/git-repository.svelte.js', () => {
  const idle = (): Given => ({ failure: null, status: 'idle', value: null });
  return {
    RepositoryPanel: class {
      actionNotice = given.actionNotice;
      activeDialog = null;
      activeTab = 'changes';
      blame = idle();
      blameFollowMoves = false;
      blameIgnoreWhitespace = false;
      branches = idle();
      chosenCommit = null;
      chosenStashIndex = null;
      commit = idle();
      compareBase = '';
      compareHead = 'WORKTREE';
      compareMode = 'three_dot';
      compareResult = idle();
      conflicts = idle();
      diffMode = 'unified';
      file = null;
      fileDiff = idle();
      fileDiffTarget = null;
      fileHistory = idle();
      fileTree = idle();
      fileViewMode = 'tree';
      graphCommits = [];
      graphHeadOid = null;
      graphMore = false;
      graphPage = idle();
      graphReflog = [];
      grepCaseSensitive = false;
      grepPath = '';
      grepPattern = '';
      grepRegex = false;
      grepResult = idle();
      history = [];
      historyAuthor = '';
      historyFirstParent = false;
      historyMerges = 'all';
      historyMore = false;
      historyPage = idle();
      historyPath = '';
      historyPickaxe = '';
      historyQuery = '';
      historyScope = { kind: 'current_branch' };
      ignoreWhitespace = false;
      inspectMode = 'blame';
      inspectPath = '';
      inspectRevision = '';
      overview = given.overview;
      pendingConfirmation = given.pendingConfirmation;
      projectId = 'project-1';
      refs = idle();
      runningAction = false;
      stashView = idle();
      watch = (): (() => void) => () => undefined;
    },
  };
});

const OVERVIEW: RepositoryOverview = {
  branch: { ahead: 2, behind: 1, head: 'main', oid: 'a1b2c3d4', upstream: 'origin/main' },
  entries: [
    {
      conflict: 'both_changed',
      kind: 'conflicted',
      path: 'src/merge.rs',
      staged: 'U',
      submodule: false,
      unstaged: 'U',
    },
  ],
  entriesTotal: 1,
  head: null,
  operation: { branch: 'feature', incoming: null, kind: 'rebase', onto: null, step: { number: 3, total: 7 } },
  pushTarget: { fetch: null, name: 'deploy', push: 'https://push.example/app.git' },
  remotes: [{ fetch: 'https://example.com/app.git', name: 'origin', push: null }],
  stagedStats: [],
  stash: 1,
  top: 'D:/Projects/app',
  unstagedStats: [],
};

function failure(code: string): unknown {
  return { error: { code, description: 'failed', origin: 'plugin' }, kind: 'core' };
}

function shown(): string {
  const props = { argument: 'project-1', close: () => undefined, setLabel: () => undefined };
  return render(GitPanel, { props }).body;
}

beforeEach(() => {
  given.actionNotice = null;
  given.overview = { failure: null, status: 'idle', value: null };
  given.pendingConfirmation = null;
});

describe('GitPanel', () => {
  it('shows that it reads the repository', () => {
    given.overview = { failure: null, status: 'loading', value: null };

    const body = shown();

    expect(body).toContain('aria-busy="true"');
    expect(body).toContain('Loading the repository');
  });

  it('says what failed and offers to try again', () => {
    given.overview = { failure: failure('git_missing'), status: 'error', value: null };

    const body = shown();

    expect(body).toContain('The repository could not be read');
    expect(body).toContain('git was not found. Install git 2.40 or later.');
    expect(body).toContain('Try again');
  });

  it('says that the project is no repository', () => {
    given.overview = { failure: failure('not_a_repository'), status: 'error', value: null };

    const body = shown();

    expect(body).toContain('is not in a git repository.');
    expect(body).not.toContain('Try again');
  });

  it('shows the branch, upstream counts, operation banner with controls, tabs and conflicted file', () => {
    given.overview = { failure: null, status: 'ready', value: OVERVIEW };

    const body = shown();

    expect(body).toContain('On main');
    expect(body).toContain('2 ahead, 1 behind origin/main');
    expect(body).toContain('A rebase is at step 3 of 7.');
    expect(body).toContain('Continue');
    expect(body).toContain('Skip');
    expect(body).toContain('Abort');
    expect(body).toContain('Graph');
    expect(body).toContain('Branches');
    expect(body).toContain('Compare');
    expect(body).toContain('Inspect');
    expect(body).toContain('Conflicts');
    expect(body).toContain('merge.rs');
    expect(body).toContain('https://push.example/app.git');
    expect(body).not.toContain('https://example.com/app.git');
    expect(body).not.toContain('role="alert"');
  });

  it('names an unborn branch clearly instead of hiding its name', () => {
    given.overview = {
      failure: null,
      status: 'ready',
      value: {
        ...OVERVIEW,
        branch: { ahead: null, behind: null, head: 'main', oid: null, upstream: null },
        entries: [{ kind: 'untracked', path: 'todo.txt', staged: '.', submodule: false, unstaged: '.' }],
        operation: null,
        stash: 0,
      },
    };

    const body = shown();

    expect(body).toContain('On main');
    expect(body).toContain('No commits yet');
    expect(body).toContain('todo.txt');
    expect(body).not.toContain('A rebase is');
  });

  it('names a refresh that failed above the repository that it shows, with "Try again"', () => {
    given.overview = { failure: failure('dubious_ownership'), status: 'error', value: OVERVIEW };

    const body = shown();

    expect(body).toContain('role="alert"');
    expect(body).toContain('git refuses this repository because another user owns its folder.');
    expect(body).toContain('Try again');
    expect(body).toContain('On main');
  });
});
