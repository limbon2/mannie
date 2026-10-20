// The Git side panel's states (plugins/git/docs/panel.md), rendered on the
// server side with the repository's state given: loading, a failure with
// "Try again", a folder that is no repository, and a repository with its
// branch, its upstream, the operation in progress, its stash and its
// changed files, with a refresh that failed named above the tabs.
import { render } from 'svelte/server';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import GitPanel from '../../../ui/components/GitPanel.svelte';
import type { RepositoryOverview } from '../../../ui/generated/bindings.js';

/** The state of a value that the panel reads, as the test gives it. */
interface Given {
  failure: unknown;
  status: string;
  value: unknown;
}

const given = vi.hoisted((): { overview: Given } => ({ overview: { failure: null, status: 'idle', value: null } }));

vi.mock('../../../ui/state/git-repository.svelte.js', () => {
  const idle = (): Given => ({ failure: null, status: 'idle', value: null });
  return {
    RepositoryPanel: class {
      branches = idle();
      chosenCommit = null;
      commit = idle();
      conflicts = idle();
      file = null;
      fileDiff = idle();
      history = [];
      historyMore = false;
      historyPage = idle();
      overview = given.overview;
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
  stash: 1,
  top: 'D:/Projects/app',
};

function failure(code: string): unknown {
  return { error: { code, description: 'failed', origin: 'plugin' }, kind: 'core' };
}

function shown(): string {
  const props = { argument: 'project-1', close: () => undefined, setLabel: () => undefined };
  return render(GitPanel, { props }).body;
}

beforeEach(() => {
  given.overview = { failure: null, status: 'idle', value: null };
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

  it('shows the branch, its upstream, the operation, the stash and the changed files', () => {
    given.overview = { failure: null, status: 'ready', value: OVERVIEW };

    const body = shown();

    expect(body).toContain('On main');
    expect(body).toContain('2 ahead, 1 behind origin/main');
    expect(body).toContain('A rebase is at step 3 of 7.');
    expect(body).toContain('1 stash entry');
    expect(body).toContain('Conflicts');
    expect(body).toContain('both changed: src/merge.rs');
    expect(body).not.toContain('role="alert"');
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
