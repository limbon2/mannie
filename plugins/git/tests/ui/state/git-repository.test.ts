// A project's repository as the Git side panel shows it (plugins/git/docs/
// panel.md): only the newest read of a value counts, a shown value stays
// through a failed read, the overview is read on a timer that waits longer
// after failures and after each change that the plugin's tools make, the
// history is read again only when HEAD moved, and a watch ends with its
// timer and its subscription.
import type * as Ipc from '@mannie/plugin-sdk/ipc';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type {
  Commit,
  CommitView,
  GitError,
  RepositoryOverview,
  RepositoryUpdate,
} from '../../../ui/generated/bindings.js';
import {
  Loaded,
  MAX_REFRESH_MILLIS,
  REFRESH_MILLIS,
  RepositoryPanel,
} from '../../../ui/state/git-repository.svelte.js';

const core = vi.hoisted(() => ({
  followRepository: vi.fn(),
  repositoryBranches: vi.fn(),
  repositoryCommit: vi.fn(),
  repositoryConflicts: vi.fn(),
  repositoryFileDiff: vi.fn(),
  repositoryHistory: vi.fn(),
  repositoryOverview: vi.fn(),
  unsubscribe: vi.fn(),
}));

/** Hands batches of messages to the panel, as the plugin's channel does. */
const channel = vi.hoisted(() => ({
  deliver: (_messages: readonly Ipc.ChannelMessage<RepositoryUpdate>[]): void => undefined,
}));

vi.mock('../../../ui/generated/bindings.js', () => ({
  channels: { followRepository: core.followRepository },
  commands: {
    repositoryBranches: core.repositoryBranches,
    repositoryCommit: core.repositoryCommit,
    repositoryConflicts: core.repositoryConflicts,
    repositoryFileDiff: core.repositoryFileDiff,
    repositoryHistory: core.repositoryHistory,
    repositoryOverview: core.repositoryOverview,
  },
}));
vi.mock('@mannie/plugin-sdk/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  batchChannel: (onItems: (messages: readonly Ipc.ChannelMessage<RepositoryUpdate>[]) => void) => {
    channel.deliver = onItems;
    return {};
  },
  unsubscribe: core.unsubscribe,
}));

function ok<T>(data: T): { data: T; status: 'ok' } {
  return { data, status: 'ok' };
}

function failed(code: GitError['code']): {
  error: { code: GitError['code']; description: string; origin: 'plugin' };
  status: 'error';
} {
  return { error: { code, description: 'failed', origin: 'plugin' }, status: 'error' };
}

function view(oid: string): CommitView {
  return {
    commit: {
      authored: '2026-10-19T14:02:11+02:00',
      author: 'Ada',
      committed: '2026-10-19T14:02:11+02:00',
      committer: 'Ada',
      committerEmail: 'ada@example.com',
      email: 'ada@example.com',
      message: `Commit ${oid}`,
      oid,
      parents: [],
      refs: [],
      short: oid,
      shortParents: [],
    },
    files: [],
    patches: [],
  };
}

function overview(head: string | null): RepositoryOverview {
  return {
    branch: { ahead: null, behind: null, head: 'main', oid: head, upstream: null },
    entries: [],
    entriesTotal: 0,
    head: null,
    operation: null,
    stash: 0,
    top: 'D:/Projects/app',
  };
}

function commit(oid: string): Commit {
  return {
    authored: '2026-10-19T14:02:11+02:00',
    author: 'Ada',
    committed: '2026-10-19T14:02:11+02:00',
    email: 'ada@example.com',
    oid,
    parents: [],
    refs: [],
    short: oid.slice(0, 7),
    subject: `Commit ${oid}`,
  };
}

/** Waits for the promises that a test started without awaiting. */
async function settle(): Promise<void> {
  for (let turn = 0; turn < 8; turn += 1) {
    await Promise.resolve();
  }
}

beforeEach(() => {
  for (const command of Object.values(core)) {
    command.mockReset();
  }
  core.followRepository.mockResolvedValue(ok(41));
  core.repositoryOverview.mockResolvedValue(ok(overview('aaa')));
  core.unsubscribe.mockResolvedValue(ok(null));
});

afterEach(() => {
  vi.useRealTimers();
});

describe('a value that the panel reads', () => {
  it('keeps the newest read when an older one answers later', async () => {
    const loaded = new Loaded<string>();
    let answerFirst: (value: { data: string; status: 'ok' }) => void = () => undefined;
    const first = loaded.load(
      () =>
        new Promise((resolve) => {
          answerFirst = resolve;
        }),
    );
    const second = loaded.load(() => Promise.resolve(ok('newer')));

    expect(await second).toBe('newer');
    answerFirst(ok('older'));

    expect(await first).toBeNull();
    expect(loaded.value).toBe('newer');
    expect(loaded.status).toBe('ready');
  });

  it('keeps a shown value through a failed read, with the failure', async () => {
    const loaded = new Loaded<string>();
    await loaded.load(() => Promise.resolve(ok('shown')));

    const read = await loaded.load(() => Promise.resolve(failed('timed_out')));

    expect(read).toBeNull();
    expect(loaded.value).toBe('shown');
    expect(loaded.status).toBe('error');
    expect(loaded.failure?.kind).toBe('core');
  });

  it('shows that it reads again after a failure while nothing shows', async () => {
    const loaded = new Loaded<string>();
    await loaded.load(() => Promise.resolve(failed('git_missing')));
    const pending = loaded.load(() => Promise.resolve(ok('found')));

    expect(loaded.status).toBe('loading');
    expect(loaded.failure).toBeNull();
    await pending;
    expect(loaded.value).toBe('found');
  });
});

describe('the repository of an open panel', () => {
  it('reads the overview on a timer that waits longer after failures', async () => {
    vi.useFakeTimers();
    const panel = new RepositoryPanel('project-1');
    const stop = panel.watch();
    await settle();
    expect(core.repositoryOverview).toHaveBeenCalledTimes(1);
    expect(core.repositoryOverview).toHaveBeenCalledWith({ projectId: 'project-1' });

    core.repositoryOverview.mockResolvedValue(failed('timed_out'));
    await vi.advanceTimersByTimeAsync(REFRESH_MILLIS);
    expect(core.repositoryOverview).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(REFRESH_MILLIS);
    expect(core.repositoryOverview).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(REFRESH_MILLIS);
    expect(core.repositoryOverview).toHaveBeenCalledTimes(3);

    stop();
    await vi.advanceTimersByTimeAsync(MAX_REFRESH_MILLIS * 2);
    expect(core.repositoryOverview).toHaveBeenCalledTimes(3);
    expect(core.unsubscribe).toHaveBeenCalledWith(41);
  });

  it('reads the overview again after each change that the plugin’s tools make', async () => {
    vi.useFakeTimers();
    const panel = new RepositoryPanel('project-1');
    const stop = panel.watch();
    await settle();

    channel.deliver([{ item: { kind: 'changed' }, kind: 'item' }]);
    await settle();
    expect(core.repositoryOverview).toHaveBeenCalledTimes(2);

    stop();
    channel.deliver([{ item: { kind: 'changed' }, kind: 'item' }]);
    await settle();
    expect(core.repositoryOverview).toHaveBeenCalledTimes(2);
  });

  it('ends the watch before it when a new one starts, and keeps the new one', async () => {
    vi.useFakeTimers();
    const panel = new RepositoryPanel('project-1');
    const stopFirst = panel.watch();
    await settle();
    const stopSecond = panel.watch();
    await settle();

    stopFirst();
    await vi.advanceTimersByTimeAsync(REFRESH_MILLIS);
    expect(core.repositoryOverview).toHaveBeenCalledTimes(3);
    expect(core.unsubscribe).toHaveBeenCalledTimes(1);

    stopSecond();
    expect(core.unsubscribe).toHaveBeenCalledTimes(2);
  });

  it('reads the history by pages, and again from the first page only when HEAD moved', async () => {
    const panel = new RepositoryPanel('project-1');
    await panel.refresh();
    core.repositoryHistory.mockResolvedValueOnce(ok({ commits: [commit('c3'), commit('c2')], more: true }));
    core.repositoryHistory.mockResolvedValueOnce(ok({ commits: [commit('c1')], more: false }));

    await panel.readHistory();
    await panel.readHistory();

    expect(core.repositoryHistory).toHaveBeenNthCalledWith(2, { projectId: 'project-1', skip: 2 });
    expect(panel.history.map((shown) => shown.oid)).toEqual(['c3', 'c2', 'c1']);
    expect(panel.historyMore).toBe(false);

    await panel.refresh();
    await settle();
    expect(core.repositoryHistory).toHaveBeenCalledTimes(2);

    core.repositoryOverview.mockResolvedValue(ok(overview('bbb')));
    core.repositoryHistory.mockResolvedValueOnce(ok({ commits: [commit('c4'), commit('c3')], more: true }));
    await panel.refresh();
    await settle();

    expect(core.repositoryHistory).toHaveBeenLastCalledWith({ projectId: 'project-1', skip: 0 });
    expect(panel.history.map((shown) => shown.oid)).toEqual(['c4', 'c3']);
    expect(panel.historyMore).toBe(true);
  });

  it('keeps the history that shows when a page fails', async () => {
    const panel = new RepositoryPanel('project-1');
    core.repositoryHistory.mockResolvedValueOnce(ok({ commits: [commit('c2')], more: true }));
    core.repositoryHistory.mockResolvedValueOnce(failed('timed_out'));

    await panel.readHistory();
    await panel.readHistory();

    expect(panel.history.map((shown) => shown.oid)).toEqual(['c2']);
    expect(panel.historyPage.failure?.kind).toBe('core');
  });

  it('reads a chosen file’s diff again with the overview, and forgets it on the way back', async () => {
    const panel = new RepositoryPanel('project-1');
    core.repositoryFileDiff.mockResolvedValue(ok({ hunks: [], stat: null }));

    panel.chooseFile('src/lib.rs', true);
    await settle();
    await panel.refresh();
    await settle();

    expect(core.repositoryFileDiff).toHaveBeenCalledTimes(2);
    expect(core.repositoryFileDiff).toHaveBeenCalledWith({ path: 'src/lib.rs', projectId: 'project-1', staged: true });
    panel.closeFile();
    expect(panel.file).toBeNull();
    expect(panel.fileDiff.value).toBeNull();
  });

  it('marks the chosen commit at once, and shows no other commit while it reads', async () => {
    const panel = new RepositoryPanel('project-1');
    core.repositoryCommit.mockResolvedValueOnce(ok(view('c1')));
    panel.chooseCommit('c1');
    await settle();
    let answer: (value: unknown) => void = () => undefined;
    core.repositoryCommit.mockReturnValueOnce(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );

    panel.chooseCommit('c2');
    const whileReading = [panel.chosenCommit, panel.commit.status, panel.commit.value];
    answer(ok(view('c2')));
    await settle();

    expect(whileReading).toEqual(['c2', 'loading', null]);
    expect(panel.commit.value?.commit.oid).toBe('c2');
    expect(core.repositoryCommit).toHaveBeenLastCalledWith({ commit: 'c2', projectId: 'project-1' });
  });

  it('reads a view again with the overview after its read failed', async () => {
    const panel = new RepositoryPanel('project-1');
    core.repositoryBranches.mockResolvedValueOnce(failed('timed_out'));
    core.repositoryBranches.mockResolvedValueOnce(ok({ branches: [], remoteBranches: [], tags: [] }));
    await panel.readBranches();
    const afterFailure = panel.branches.status;

    await panel.refresh();
    await settle();

    expect(afterFailure).toBe('error');
    expect(panel.branches.status).toBe('ready');
    expect(panel.branches.failure).toBeNull();
  });

  it('reads the branches and the conflicts again with the overview once they show', async () => {
    const panel = new RepositoryPanel('project-1');
    core.repositoryBranches.mockResolvedValue(ok({ branches: [], remoteBranches: [], tags: [] }));
    core.repositoryConflicts.mockResolvedValue(ok({ files: [], head: null, operation: null }));
    await panel.refresh();
    expect(core.repositoryBranches).not.toHaveBeenCalled();

    await panel.readBranches();
    await panel.readConflicts();
    await panel.refresh();
    await settle();

    expect(core.repositoryBranches).toHaveBeenCalledTimes(2);
    expect(core.repositoryConflicts).toHaveBeenCalledTimes(2);
  });
});
