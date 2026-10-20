// A project's repository as the Git side panel shows it (plugins/git/docs/
// panel.md). The overview, its branch, operation and changed files, is read
// when the panel opens, after each change that the plugin's tools make, on
// Refresh, and every 15 seconds while the panel shows, waiting longer after
// each failure. The history, the branches, the conflicts, a commit and a
// file's diff are read when the panel asks for them, and read again with the
// overview once they show. Only the newest read of each value counts, so a
// slow answer never replaces a newer one.
import {
  batchChannel,
  call,
  type ChannelMessage,
  type CommandError,
  type PluginFailure,
  type Result,
  type SubscriptionId,
  unsubscribe,
} from '@mannie/plugin-sdk/ipc';

import {
  type BranchList,
  channels,
  commands,
  type Commit,
  type CommitView,
  type ConflictsView,
  type FileDiff,
  type GitError,
  type History,
  type ProjectId,
  type RepositoryOverview,
  type RepositoryUpdate,
} from '../generated/bindings.js';
import type { GitFailure } from '../types/git.js';

/** How long the overview waits between reads while the panel shows. */
export const REFRESH_MILLIS = 15_000;

/** The longest wait between reads after failures. */
export const MAX_REFRESH_MILLIS = 120_000;

/** What a command of the plugin answers. */
type GitResult<T> = Result<T, CommandError<PluginFailure<GitError>>>;

/** A value that a command reads, with the state of its newest read. */
export class Loaded<T> {
  /** Why the newest read failed, set exactly while `status` is `'error'`. */
  failure: GitFailure | null = $state(null);
  status: 'error' | 'idle' | 'loading' | 'ready' = $state('idle');
  value: T | null = $state.raw(null);
  #generation = 0;

  /** Forgets the value, as when what it was read for changes. */
  clear(): void {
    this.#generation += 1;
    this.value = null;
    this.failure = null;
    this.status = 'idle';
  }

  /**
   * Reads the value with `read`, and returns it when this read is the
   * newest and succeeded. A value that is shown stays until a newer read
   * succeeds, with the failure of a read that failed.
   */
  async load(read: () => Promise<GitResult<T>>): Promise<T | null> {
    this.#generation += 1;
    const generation = this.#generation;
    if (this.value === null) {
      this.status = 'loading';
      this.failure = null;
    }
    const outcome = await call(read);
    if (generation !== this.#generation) {
      return null;
    }
    if (outcome.ok) {
      this.value = outcome.data;
      this.failure = null;
      this.status = 'ready';
      return outcome.data;
    }
    this.failure = outcome.failure;
    this.status = 'error';
    return null;
  }
}

/** The repository of one project, for one open panel. */
export class RepositoryPanel {
  readonly branches = new Loaded<BranchList>();
  /** The commit whose changes are chosen. */
  chosenCommit: string | null = $state(null);
  readonly commit = new Loaded<CommitView>();
  readonly conflicts = new Loaded<ConflictsView>();
  readonly fileDiff = new Loaded<FileDiff>();
  /** The file whose diff is chosen, and whether its staged change. */
  file: { readonly path: string; readonly staged: boolean } | null = $state.raw(null);
  /** The commits of the history that are read, newest first. */
  history: readonly Commit[] = $state.raw([]);
  /** Whether older commits follow the ones that are read. */
  historyMore = $state(false);
  readonly historyPage = new Loaded<History>();
  readonly overview = new Loaded<RepositoryOverview>();
  readonly projectId: ProjectId;
  /** The commit of HEAD that the history's first page was read at. */
  #historyHead: string | null = null;
  #subscription: SubscriptionId | null = null;
  #timer: ReturnType<typeof setTimeout> | null = null;
  #wait = REFRESH_MILLIS;
  /** The number of the watch that runs, or 0 while none does. */
  #watch = 0;
  #watches = 0;

  constructor(projectId: ProjectId) {
    this.projectId = projectId;
  }

  /** Reads the commit `oid` for its changes, in place of the commit that shows. */
  chooseCommit(oid: string): void {
    this.chosenCommit = oid;
    this.commit.clear();
    this.readCommit();
  }

  /** Reads the change of the file at `path`, staged or not. */
  chooseFile(path: string, staged: boolean): void {
    this.file = { path, staged };
    this.readFile();
  }

  /** Goes back from a file's diff to the changed files. */
  closeFile(): void {
    this.file = null;
    this.fileDiff.clear();
  }

  /**
   * Reads the overview again, then every view that was opened, such as
   * after its read failed. The history is read again from its first page
   * when HEAD moved since it was read.
   */
  async refresh(): Promise<void> {
    const overview = await this.overview.load(() => commands.repositoryOverview({ projectId: this.projectId }));
    if (overview === null) {
      return;
    }
    if (this.branches.status !== 'idle') {
      void this.readBranches();
    }
    if (this.conflicts.status !== 'idle') {
      void this.readConflicts();
    }
    this.readFile();
    const isHistoryRead = this.historyPage.status !== 'idle';
    const hasHeadMoved = overview.branch.oid !== this.#historyHead;
    if (isHistoryRead && hasHeadMoved) {
      void this.#readFirstPage(overview.branch.oid);
    }
  }

  /** Reads the branches. */
  async readBranches(): Promise<void> {
    await this.branches.load(() => commands.repositoryBranches({ projectId: this.projectId }));
  }

  /** Reads the chosen commit, if one is chosen. */
  readCommit(): void {
    const oid = this.chosenCommit;
    if (oid !== null) {
      void this.commit.load(() => commands.repositoryCommit({ commit: oid, projectId: this.projectId }));
    }
  }

  /** Reads the conflicts. */
  async readConflicts(): Promise<void> {
    await this.conflicts.load(() => commands.repositoryConflicts({ projectId: this.projectId }));
  }

  /** Reads the chosen file's change, if a file is chosen. */
  readFile(): void {
    const file = this.file;
    if (file !== null) {
      void this.fileDiff.load(() =>
        commands.repositoryFileDiff({ path: file.path, projectId: this.projectId, staged: file.staged }),
      );
    }
  }

  /** Reads the history's first page, or the page after the commits that are read. */
  async readHistory(): Promise<void> {
    if (this.history.length === 0) {
      await this.#readFirstPage(this.overview.value?.branch.oid ?? null);
      return;
    }
    const skip = this.history.length;
    const page = await this.historyPage.load(() => commands.repositoryHistory({ projectId: this.projectId, skip }));
    if (page !== null) {
      this.history = [...this.history, ...page.commits];
      this.historyMore = page.more;
    }
  }

  /**
   * Reads the overview now, then while the panel shows: after each change
   * that the plugin's tools make and on a timer. A new watch ends the one
   * before it. Returns what ends this one.
   */
  watch(): () => void {
    this.#stop();
    this.#watches += 1;
    const watch = this.#watches;
    this.#watch = watch;
    this.#wait = REFRESH_MILLIS;
    void this.refresh();
    this.#schedule(watch);
    void this.#follow(watch);
    return () => {
      if (this.#isWatching(watch)) {
        this.#stop();
      }
    };
  }

  async #follow(watch: number): Promise<void> {
    const channel = batchChannel<ChannelMessage<RepositoryUpdate>>(
      (messages) => {
        const isChanged = messages.some((message) => message.kind === 'item');
        if (isChanged && this.#isWatching(watch)) {
          void this.refresh();
        }
      },
      () => {
        if (this.#isWatching(watch)) {
          void this.#follow(watch);
        }
      },
    );
    const outcome = await call(() => channels.followRepository(channel));
    if (!outcome.ok) {
      // The timer still reads the repository; the panel only misses the
      // quick reads after the tools' changes.
      return;
    }
    if (this.#isWatching(watch)) {
      this.#subscription = outcome.data;
    } else {
      void unsubscribe(outcome.data);
    }
  }

  /** Whether the watch `watch` still runs. */
  #isWatching(watch: number): boolean {
    return this.#watch === watch;
  }

  async #readFirstPage(head: string | null): Promise<void> {
    const page = await this.historyPage.load(() => commands.repositoryHistory({ projectId: this.projectId, skip: 0 }));
    if (page !== null) {
      this.#historyHead = head;
      this.history = page.commits;
      this.historyMore = page.more;
    }
  }

  #schedule(watch: number): void {
    this.#timer = setTimeout(() => {
      void this.refresh().then(() => {
        if (!this.#isWatching(watch)) {
          return;
        }
        const isFailing = this.overview.status === 'error';
        this.#wait = isFailing ? Math.min(this.#wait * 2, MAX_REFRESH_MILLIS) : REFRESH_MILLIS;
        this.#schedule(watch);
      });
    }, this.#wait);
  }

  #stop(): void {
    this.#watch = 0;
    if (this.#timer !== null) {
      clearTimeout(this.#timer);
      this.#timer = null;
    }
    const subscription = this.#subscription;
    this.#subscription = null;
    if (subscription !== null) {
      void unsubscribe(subscription);
    }
  }
}
