// A project's repository as the Git side panel shows it (plugins/git/docs/
// panel.md). Manages generation-guarded reads, live refresh via
// `follow_repository` and adaptive polling, cross-tab navigation, chat
// mentions (ADR-0054), and journaled owner actions with server-enforced
// confirmation (ADR-0078).
import { formatCount } from '@mannie/plugin-sdk/format';
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
import { mention } from '@mannie/plugin-sdk/message-box';
import { openConversationId } from '@mannie/plugin-sdk/navigation';

import {
  type ActionConfirmation,
  type ActionPreview,
  type BlameView,
  type BranchList,
  channels,
  commands,
  type Commit,
  type CommitView,
  type CompareMode,
  type CompareView,
  type ConflictsView,
  type FileDiff,
  type FileTreeList,
  type GitError,
  type GrepView,
  type History,
  type HistoryQueryResult,
  type HistoryScope,
  type MergeFilter,
  type OwnerActionResult,
  type OwnerGitAction,
  type ProjectId,
  type ReflogEntry,
  type RefsOverview,
  type RepositoryOverview,
  type RepositoryUpdate,
  type StashView,
} from '../generated/bindings.js';
import { m } from '../generated/messages.js';
import type { GitFailure } from '../types/git.js';
import type { GitMentionEntry } from '../utils/git-format.js';
import { gitFailureMessage } from '../utils/git-messages.js';

/** How long the overview waits between reads while the panel shows. */
export const REFRESH_MILLIS = 15_000;

/** The longest wait between reads after failures. */
export const MAX_REFRESH_MILLIS = 120_000;

/** Primary tabs inside the Git side panel. */
export type GitPanelTab = 'branches' | 'changes' | 'compare' | 'conflicts' | 'graph' | 'inspect';

/** Sub-modes inside the Inspect tab. */
export type InspectMode = 'blame' | 'grep' | 'history';

/** Transient status banner shown after an owner action or mention. */
export interface ActionNotice {
  readonly detail?: string;
  readonly kind: 'error' | 'info';
  readonly message: string;
}

/** Pending owner action awaiting explicit confirmation. */
export interface PendingConfirmation {
  readonly action: OwnerGitAction;
  readonly preview: ActionPreview;
}

/** Active modal dialog for creating or renaming Git objects. */
export type ActiveGitDialog =
  | { readonly kind: 'create_branch'; readonly start: string }
  | { readonly kind: 'create_tag'; readonly target: string }
  | { readonly kind: 'rename_branch'; readonly name: string }
  | { readonly kind: 'stash_push' };

/** What a command of the plugin answers. */
type GitResult<T> = Result<T, CommandError<PluginFailure<GitError>>>;

/** The file whose diff is currently open in the Changes tab. */
export interface SelectedFile {
  readonly path: string;
  readonly staged: boolean;
}

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
  actionNotice: ActionNotice | null = $state.raw(null);
  activeDialog: ActiveGitDialog | null = $state.raw(null);
  activeTab: GitPanelTab = $state('changes');
  readonly blame = new Loaded<BlameView>();
  blameFollowMoves = $state(false);
  blameIgnoreWhitespace = $state(false);
  readonly branches = new Loaded<BranchList>();
  /** The commit whose changes are chosen. */
  chosenCommit: string | null = $state(null);
  chosenStashIndex: number | null = $state(null);
  readonly commit = new Loaded<CommitView>();
  compareBase = $state('');
  compareHead = $state('WORKTREE');
  compareMode: CompareMode = $state('three_dot');
  readonly compareResult = new Loaded<CompareView>();
  readonly conflicts = new Loaded<ConflictsView>();
  diffMode: 'split' | 'unified' = $state('unified');
  /** The file whose diff is chosen, and whether its staged change. */
  file: SelectedFile | null = $state.raw(null);
  readonly fileDiff = new Loaded<FileDiff>();
  /** The selection that `fileDiff.value` was actually loaded for. */
  fileDiffTarget: SelectedFile | null = $state.raw(null);
  readonly fileHistory = new Loaded<HistoryQueryResult>();
  readonly fileTree = new Loaded<FileTreeList>();
  fileViewMode: 'list' | 'tree' = $state('tree');
  graphCommits: readonly Commit[] = $state.raw([]);
  graphHeadOid: string | null = $state(null);
  graphMore = $state(false);
  readonly graphPage = new Loaded<HistoryQueryResult>();
  graphReflog: readonly ReflogEntry[] = $state.raw([]);
  grepCaseSensitive = $state(false);
  grepPath = $state('');
  grepPattern = $state('');
  grepRegex = $state(false);
  readonly grepResult = new Loaded<GrepView>();
  /** The commits of the legacy history stream, newest first. */
  history: readonly Commit[] = $state.raw([]);
  historyAuthor = $state('');
  historyFirstParent = $state(false);
  historyMerges: MergeFilter = $state('all');
  /** Whether older commits follow the ones that are read. */
  historyMore = $state(false);
  readonly historyPage = new Loaded<History>();
  historyPath = $state('');
  historyPickaxe = $state('');
  historyQuery = $state('');
  historyScope: HistoryScope = $state.raw({ kind: 'current_branch' });
  ignoreWhitespace = $state(false);
  inspectMode: InspectMode = $state('blame');
  inspectPath = $state('');
  inspectRevision = $state('');
  readonly overview = new Loaded<RepositoryOverview>();
  pendingConfirmation: PendingConfirmation | null = $state.raw(null);
  readonly projectId: ProjectId;
  readonly refs = new Loaded<RefsOverview>();
  runningAction = $state(false);
  readonly stashView = new Loaded<StashView>();
  #graphHead: string | null = null;
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

  /** Cancels the confirmation dialog without running the action. */
  cancelPendingAction(): void {
    this.pendingConfirmation = null;
  }

  /** Reads the commit `oid` for its changes, in place of the commit that shows. */
  chooseCommit(oid: string): void {
    this.chosenCommit = oid;
    this.commit.clear();
    this.readCommit();
  }

  /** Reads the change of the file at `path`, staged or not. */
  chooseFile(path: string, staged: boolean): void {
    const isSame = this.file !== null && this.file.path === path && this.file.staged === staged;
    this.file = { path, staged };
    if (!isSame) {
      this.fileDiff.clear();
      this.fileDiffTarget = null;
    }
    this.readFile();
  }

  /** Reads the diff of stash entry `index`. */
  chooseStash(index: number): void {
    this.chosenStashIndex = index;
    this.stashView.clear();
    void this.readStash();
  }

  /** Resets all graph filters and reloads the commit graph. */
  clearGraphFilters(): void {
    this.historyQuery = '';
    this.historyAuthor = '';
    this.historyPath = '';
    this.historyPickaxe = '';
    this.historyMerges = 'all';
    this.historyFirstParent = false;
    void this.readGraph(true);
  }

  /** Closes the commit that shows, as choosing it again does. */
  closeCommit(): void {
    this.chosenCommit = null;
    this.commit.clear();
  }

  /** Closes any active creation/rename dialog. */
  closeDialog(): void {
    this.activeDialog = null;
  }

  /** Goes back from a file's diff to the changed files. */
  closeFile(): void {
    this.file = null;
    this.fileDiff.clear();
    this.fileDiffTarget = null;
  }

  /** Closes the open stash diff view. */
  closeStash(): void {
    this.chosenStashIndex = null;
    this.stashView.clear();
  }

  /** Runs the action currently awaiting owner confirmation. */
  async confirmPendingAction(): Promise<void> {
    const pending = this.pendingConfirmation;
    if (pending === null) {
      return;
    }
    this.pendingConfirmation = null;
    await this.runAction(pending.action, {
      effectsDigest: pending.preview.effectsDigest,
      state: pending.preview.state,
    });
  }

  /** Clears the active action notice banner. */
  dismissNotice(): void {
    this.actionNotice = null;
  }

  /** Loads the next page of commits or reflog entries in the Graph tab. */
  async loadMoreGraph(): Promise<void> {
    await this.readGraph(false);
  }

  /** Adds a Git mention (commit, branch or uncommitted changes) into the open chat message box. */
  mentionInChat(entry: GitMentionEntry): void {
    const conversationId = openConversationId();
    if (conversationId === null) {
      this.actionNotice = {
        kind: 'info',
        message: m.git_panel_mention_needs_conversation(),
      };
      return;
    }
    const outcome = mention(conversationId, { entry, provider: 'git' });
    if (outcome === 'added') {
      this.actionNotice = {
        kind: 'info',
        message: m.git_panel_mentioned_in_chat(),
      };
    } else if (outcome === 'not_shown') {
      this.actionNotice = {
        kind: 'info',
        message: m.git_panel_mention_needs_conversation(),
      };
    }
  }

  /** Jumps to the Graph tab and inspects `oid`. */
  openCommitInGraph(oid: string): void {
    this.activeTab = 'graph';
    if (this.graphPage.status === 'idle') {
      void this.readGraph(true);
    }
    this.chooseCommit(oid);
  }

  /** Jumps to the Compare tab and compares `base` against `head`. */
  openCompare(base: string, head: string): void {
    this.compareBase = base;
    this.compareHead = head;
    this.activeTab = 'compare';
    void this.runCompare();
  }

  /** Opens a modal dialog for branch, tag or stash creation. */
  openDialog(dialog: ActiveGitDialog): void {
    this.activeDialog = dialog;
  }

  /** Jumps to Inspect -> Blame for `path` at optional `revision`. */
  openFileBlame(path: string, revision = ''): void {
    this.inspectPath = path;
    this.inspectRevision = revision;
    this.inspectMode = 'blame';
    this.activeTab = 'inspect';
    void this.readBlame();
  }

  /** Jumps to Inspect -> File History for `path`. */
  openFileHistory(path: string): void {
    this.inspectPath = path;
    this.inspectMode = 'history';
    this.activeTab = 'inspect';
    void this.readFileHistory();
  }

  /** Reads blame annotations for `inspectPath`. */
  async readBlame(): Promise<void> {
    const path = this.inspectPath.trim();
    if (path.length === 0) {
      this.blame.clear();
      return;
    }
    const revision = this.inspectRevision.trim();
    await this.blame.load(() =>
      commands.repositoryBlame({
        followMoves: this.blameFollowMoves,
        ignoreWhitespace: this.blameIgnoreWhitespace,
        path,
        projectId: this.projectId,
        revision: revision.length > 0 ? revision : null,
      }),
    );
  }

  /** Reads the legacy branches list. */
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
      const request = this.ignoreWhitespace
        ? { ignoreWhitespace: true, path: file.path, projectId: this.projectId, staged: file.staged }
        : { path: file.path, projectId: this.projectId, staged: file.staged };
      void this.fileDiff
        .load(() => commands.repositoryFileDiff(request))
        .then((loaded) => {
          if (loaded !== null) {
            this.fileDiffTarget = file;
          }
        });
    }
  }

  /** Reads commit history for `inspectPath` with `--follow`. */
  async readFileHistory(): Promise<void> {
    const path = this.inspectPath.trim();
    if (path.length === 0) {
      this.fileHistory.clear();
      return;
    }
    await this.fileHistory.load(() =>
      commands.repositoryHistoryQuery({
        limit: 100,
        path,
        projectId: this.projectId,
        scope: { kind: 'current_branch' },
      }),
    );
  }

  /** Reads tracked repository file paths matching an optional filter. */
  async readFileTree(query = ''): Promise<void> {
    const trimmed = query.trim();
    const revision = this.inspectRevision.trim();
    await this.fileTree.load(() =>
      commands.repositoryTree({
        projectId: this.projectId,
        query: trimmed.length > 0 ? trimmed : null,
        revision: revision.length > 0 ? revision : null,
      }),
    );
  }

  /** Reads commits or reflog entries for the Graph tab. */
  async readGraph(reset = true): Promise<void> {
    const isReflog = this.historyScope.kind === 'reflog';
    const loadedCount = isReflog ? this.graphReflog.length : this.graphCommits.length;
    const skip = reset ? 0 : loadedCount;
    const page = await this.graphPage.load(() =>
      commands.repositoryHistoryQuery({
        author: this.historyAuthor.trim() || null,
        firstParent: this.historyFirstParent,
        merges: this.historyMerges,
        path: this.historyPath.trim() || null,
        pickaxe: this.historyPickaxe.trim() || null,
        projectId: this.projectId,
        query: this.historyQuery.trim() || null,
        scope: this.historyScope,
        skip,
      }),
    );
    if (page !== null) {
      this.#graphHead = this.overview.value?.branch.oid ?? page.headOid;
      this.graphHeadOid = page.headOid;
      this.graphMore = page.more;
      if (isReflog) {
        this.graphCommits = [];
        this.graphReflog = reset ? page.reflog : [...this.graphReflog, ...page.reflog];
      } else {
        this.graphReflog = [];
        this.graphCommits = reset ? page.commits : [...this.graphCommits, ...page.commits];
      }
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

  /** Reads full refs, stashes, remotes and worktrees for the Branches tab and branch picker. */
  async readRefs(): Promise<void> {
    await this.refs.load(() => commands.repositoryRefs({ projectId: this.projectId }));
  }

  /** Reads the chosen stash entry's diff, if one is chosen. */
  async readStash(): Promise<void> {
    const index = this.chosenStashIndex;
    if (index === null) {
      return;
    }
    await this.stashView.load(() => commands.repositoryStashShow({ index, projectId: this.projectId }));
  }

  /**
   * Reads the overview again, then every view that was opened, such as
   * after its read failed. History and graph reload when HEAD moved.
   */
  async refresh(): Promise<void> {
    const overview = await this.overview.load(() => commands.repositoryOverview({ projectId: this.projectId }));
    if (overview === null) {
      return;
    }
    if (this.branches.status !== 'idle') {
      void this.readBranches();
    }
    if (this.refs.status !== 'idle') {
      void this.readRefs();
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
    const isGraphRead = this.graphPage.status !== 'idle';
    const hasGraphHeadMoved = overview.branch.oid !== this.#graphHead;
    if (isGraphRead && hasGraphHeadMoved) {
      void this.readGraph(true);
    }
  }

  /** Runs a journaled owner Git action (ADR-0078). */
  async runAction(
    action: OwnerGitAction,
    confirmation: ActionConfirmation | null = null,
  ): Promise<OwnerActionResult | null> {
    if (this.runningAction) {
      return null;
    }
    this.runningAction = true;
    this.actionNotice = null;
    try {
      const outcome = await call(() =>
        commands.repositoryActionRun({
          action,
          confirmation,
          operationId: crypto.randomUUID(),
          projectId: this.projectId,
        }),
      );
      if (!outcome.ok) {
        this.actionNotice = {
          kind: 'error',
          message: gitFailureMessage(outcome.failure),
        };
        return null;
      }

      const result = outcome.data;
      if (result.outcome === 'needs_confirmation') {
        this.pendingConfirmation = { action, preview: result.preview };
        return result;
      }
      if (result.outcome === 'refused') {
        this.actionNotice = {
          detail: result.detail,
          kind: 'error',
          message: m.git_error_operation_refused(),
        };
        await this.refresh();
        return result;
      }
      if (result.outcome === 'stopped') {
        this.actionNotice = {
          kind: 'info',
          message: m.git_action_stopped_conflicts({ count: formatCount(result.conflicts) }),
        };
        const hasConflicts = result.conflicts > 0;
        if (hasConflicts) {
          this.activeTab = 'conflicts';
        }
        await this.refresh();
        const isConflictsIdle = this.conflicts.status === 'idle';
        if (hasConflicts && isConflictsIdle) {
          void this.readConflicts();
        }
        return result;
      }

      if (result.snapshot !== null) {
        this.actionNotice = {
          kind: 'info',
          message: m.git_action_snapshot_saved({ oid: result.snapshot.slice(0, 7) }),
        };
      }
      await this.refresh();
      if (this.refs.status !== 'idle') {
        await this.readRefs();
      }
      if (this.graphPage.status !== 'idle') {
        await this.readGraph(true);
      }
      return result;
    } finally {
      this.runningAction = false;
    }
  }

  /** Runs a ref or worktree comparison for the Compare tab. */
  async runCompare(): Promise<void> {
    const base = this.compareBase.trim();
    const head = this.compareHead.trim();
    const isBaseEmpty = base.length === 0;
    const isHeadEmpty = head.length === 0;
    if (isBaseEmpty || isHeadEmpty) {
      this.compareResult.clear();
      return;
    }
    await this.compareResult.load(() =>
      commands.repositoryCompare({
        base,
        head,
        ignoreWhitespace: this.ignoreWhitespace,
        mode: this.compareMode,
        projectId: this.projectId,
      }),
    );
  }

  /** Searches code across tracked files or a specific revision (`git grep`). */
  async runGrep(): Promise<void> {
    const pattern = this.grepPattern.trim();
    if (pattern.length === 0) {
      this.grepResult.clear();
      return;
    }
    const revision = this.inspectRevision.trim();
    const path = this.grepPath.trim();
    await this.grepResult.load(() =>
      commands.repositoryGrep({
        caseSensitive: this.grepCaseSensitive,
        fixedStrings: !this.grepRegex,
        path: path.length > 0 ? path : null,
        pattern,
        projectId: this.projectId,
        revision: revision.length > 0 ? revision : null,
      }),
    );
  }

  /** Swaps base and head refs in the Compare tab when head is not WORKTREE. */
  swapCompare(): void {
    if (this.compareHead === 'WORKTREE') {
      return;
    }
    const previousBase = this.compareBase;
    this.compareBase = this.compareHead;
    this.compareHead = previousBase;
    void this.runCompare();
  }

  /** Switches diff whitespace handling and re-reads active diffs. */
  toggleIgnoreWhitespace(): void {
    this.ignoreWhitespace = !this.ignoreWhitespace;
    this.readFile();
    if (this.compareResult.status !== 'idle') {
      void this.runCompare();
    }
  }

  /**
   * Reads the overview now, then while the panel shows: after each change
   * that the plugin's tools or owner actions make and on a timer. A new
   * watch ends the one before it. Returns what ends this one.
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
