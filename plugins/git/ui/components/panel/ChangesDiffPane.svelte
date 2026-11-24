<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import ArrowLeftIcon from '@mannie/plugin-sdk/icons/arrow-left';
  import CheckIcon from '@mannie/plugin-sdk/icons/check';
  import ChevronLeftIcon from '@mannie/plugin-sdk/icons/chevron-left';
  import ChevronRightIcon from '@mannie/plugin-sdk/icons/chevron-right';
  import Columns2Icon from '@mannie/plugin-sdk/icons/columns-2';
  import CopyIcon from '@mannie/plugin-sdk/icons/copy';
  import FileClockIcon from '@mannie/plugin-sdk/icons/file-clock';
  import MinusIcon from '@mannie/plugin-sdk/icons/minus';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';
  import Rows2Icon from '@mannie/plugin-sdk/icons/rows-2';
  import ScanSearchIcon from '@mannie/plugin-sdk/icons/scan-search';
  import SpaceIcon from '@mannie/plugin-sdk/icons/space';
  import TrashIcon from '@mannie/plugin-sdk/icons/trash';
  import Undo2Icon from '@mannie/plugin-sdk/icons/undo-2';

  import type { Entry, Hunk, RepositoryOverview } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';
  import type { RepositoryPanel } from '../../state/git-repository.svelte.js';
  import { gitFailureMessage } from '../../utils/git-messages.js';
  import { buildHunkPatch } from '../../utils/git-word-diff.js';
  import HunkLines from '../cards/HunkLines.svelte';

  // Diff viewer pane inside ChangesTab: shows unified/split hunks with
  // untracked preview, ignore-whitespace, prev/next file stepping, and
  // per-file / per-hunk stage, unstage and discard controls.
  interface Props {
    readonly overview: RepositoryOverview;
    readonly panel: RepositoryPanel;
    readonly wide: boolean;
  }

  interface NavigableFile {
    readonly path: string;
    readonly staged: boolean;
    readonly untracked: boolean;
  }

  let { overview, panel, wide }: Props = $props();

  let copiedPath = $state(false);

  const navigableFiles: readonly NavigableFile[] = $derived.by(() => {
    const items: NavigableFile[] = [];
    for (const entry of overview.entries) {
      if (isStagedEntry(entry)) {
        items.push({ path: entry.path, staged: true, untracked: false });
      }
    }
    for (const entry of overview.entries) {
      if (isUnstagedEntry(entry)) {
        items.push({ path: entry.path, staged: false, untracked: false });
      }
    }
    for (const entry of overview.entries) {
      if (entry.kind === 'untracked') {
        items.push({ path: entry.path, staged: false, untracked: true });
      }
    }
    return items;
  });

  const currentIndex = $derived.by(() => {
    const selected = panel.file;
    if (selected === null) {
      return -1;
    }
    return navigableFiles.findIndex((item) => {
      const isSamePath = item.path === selected.path;
      const isSameStage = item.staged === selected.staged;
      return isSamePath && isSameStage;
    });
  });

  const currentFileEntry = $derived(currentIndex >= 0 ? (navigableFiles.at(currentIndex) ?? null) : null);
  const diff = $derived(panel.fileDiff.value);
  const isUntracked = $derived(diff?.untracked ?? currentFileEntry?.untracked ?? false);
  const hasFileNavigation = $derived.by(() => {
    const hasMultiple = navigableFiles.length > 1;
    const isIndexed = currentIndex >= 0;
    return hasMultiple && isIndexed;
  });
  const isLoadingInitial = $derived(panel.fileDiff.status === 'loading' && diff === null);
  const loadFailure = $derived(panel.fileDiff.status === 'error' ? panel.fileDiff.failure : null);
  const loadedTarget = $derived(panel.fileDiffTarget);
  const isDiffForSelectedFile = $derived.by(() => {
    const selected = panel.file;
    if (selected === null) {
      return false;
    }
    if (loadedTarget === null) {
      return false;
    }
    if (diff === null) {
      return false;
    }
    const isReady = panel.fileDiff.status === 'ready';
    const isSamePath = loadedTarget.path === selected.path;
    const isSameStage = loadedTarget.staged === selected.staged;
    return isReady && isSamePath && isSameStage;
  });
  const canApplyHunk = $derived(isDiffForSelectedFile && !panel.ignoreWhitespace);
  const canStageOrDiscardHunk = $derived.by(() => {
    if (!canApplyHunk) {
      return false;
    }
    if (loadedTarget === null) {
      return false;
    }
    return !loadedTarget.staged && !isUntracked;
  });
  const canUnstageHunk = $derived.by(() => {
    if (!canApplyHunk) {
      return false;
    }
    if (loadedTarget === null) {
      return false;
    }
    return loadedTarget.staged;
  });

  function isTrackedEntry(entry: Entry): boolean {
    const isConflicted = entry.kind === 'conflicted';
    const isUntrackedKind = entry.kind === 'untracked';
    const isIgnored = entry.kind === 'ignored';
    return !isConflicted && !isUntrackedKind && !isIgnored;
  }

  function isStagedEntry(entry: Entry): boolean {
    const hasIndexChange = entry.staged !== '.';
    return isTrackedEntry(entry) && hasIndexChange;
  }

  function isUnstagedEntry(entry: Entry): boolean {
    const hasWorktreeChange = entry.unstaged !== '.';
    return isTrackedEntry(entry) && hasWorktreeChange;
  }

  function stepFile(delta: number): void {
    const next = navigableFiles.at(currentIndex + delta);
    if (next !== undefined) {
      panel.chooseFile(next.path, next.staged);
    }
  }

  function toggleDiffMode(): void {
    panel.diffMode = panel.diffMode === 'unified' ? 'split' : 'unified';
  }

  function copyPath(): void {
    const path = panel.file?.path;
    if (path === undefined) {
      return;
    }
    void navigator.clipboard.writeText(path).then(() => {
      copiedPath = true;
      setTimeout(() => {
        copiedPath = false;
      }, 1500);
    });
  }

  function stageSelectedFile(): void {
    if (panel.file === null) {
      return;
    }
    void panel.runAction({ all: false, kind: 'stage_files', paths: [panel.file.path] });
  }

  function unstageSelectedFile(): void {
    if (panel.file === null) {
      return;
    }
    void panel.runAction({ all: false, kind: 'unstage_files', paths: [panel.file.path] });
  }

  function discardSelectedFile(): void {
    if (panel.file === null) {
      return;
    }
    void panel.runAction({ includeUntracked: isUntracked, kind: 'discard_files', paths: [panel.file.path] });
  }

  function stageHunk(hunk: Hunk): void {
    const target = panel.fileDiffTarget;
    if (!canStageOrDiscardHunk) {
      return;
    }
    if (target === null) {
      return;
    }
    const original = diff?.stat?.original ?? null;
    void panel.runAction({
      kind: 'stage_hunk',
      patch: buildHunkPatch(target.path, hunk, original),
      path: target.path,
    });
  }

  function unstageHunk(hunk: Hunk): void {
    const target = panel.fileDiffTarget;
    if (!canUnstageHunk) {
      return;
    }
    if (target === null) {
      return;
    }
    const original = diff?.stat?.original ?? null;
    void panel.runAction({
      kind: 'unstage_hunk',
      patch: buildHunkPatch(target.path, hunk, original),
      path: target.path,
    });
  }

  function discardHunk(hunk: Hunk): void {
    const target = panel.fileDiffTarget;
    if (!canStageOrDiscardHunk) {
      return;
    }
    if (target === null) {
      return;
    }
    const original = diff?.stat?.original ?? null;
    void panel.runAction({
      kind: 'discard_hunk',
      patch: buildHunkPatch(target.path, hunk, original),
      path: target.path,
    });
  }

  function openBlame(): void {
    if (panel.file !== null) {
      panel.openFileBlame(panel.file.path);
    }
  }

  function openHistory(): void {
    if (panel.file !== null) {
      panel.openFileHistory(panel.file.path);
    }
  }
</script>

{#if panel.file === null}
  <div class="flex h-full items-center justify-center p-4 text-center text-xs text-muted-foreground">
    {m.git_panel_choose_file()}
  </div>
{:else}
  <div class="flex h-full min-h-0 flex-col overflow-hidden" data-git-diff="">
    <div class="flex flex-wrap items-center gap-1 border-b bg-muted/30 px-2.5 py-1.5 text-xs">
      {#if !wide}
        <Button class="h-6 px-1.5 text-xs" size="sm" variant="ghost" onclick={() => panel.closeFile()}>
          <ArrowLeftIcon class="size-3.5" aria-hidden="true" />
          <span>{m.git_panel_back()}</span>
        </Button>
      {/if}

      <span class="min-w-0 flex-1 truncate font-mono font-medium" title={panel.file.path}>
        {panel.file.path}
      </span>

      {#if panel.file.staged}
        <Badge class="h-4 px-1.5 text-[10px]" variant="secondary">{m.git_status_staged()}</Badge>
      {:else if isUntracked}
        <Badge class="h-4 px-1.5 text-[10px]" variant="outline">{m.git_changes_untracked_badge()}</Badge>
      {/if}

      {#if hasFileNavigation}
        <div class="flex items-center gap-0.5">
          <Button
            class="size-6"
            aria-label={m.git_changes_prev_file()}
            disabled={currentIndex <= 0}
            size="icon"
            title={m.git_changes_prev_file()}
            variant="ghost"
            onclick={() => stepFile(-1)}
          >
            <ChevronLeftIcon class="size-3.5" aria-hidden="true" />
          </Button>
          <span class="px-1 text-[11px] text-muted-foreground tabular-nums">
            {m.git_changes_file_position({
              current: formatCount(currentIndex + 1),
              total: formatCount(navigableFiles.length),
            })}
          </span>
          <Button
            class="size-6"
            aria-label={m.git_changes_next_file()}
            disabled={currentIndex >= navigableFiles.length - 1}
            size="icon"
            title={m.git_changes_next_file()}
            variant="ghost"
            onclick={() => stepFile(1)}
          >
            <ChevronRightIcon class="size-3.5" aria-hidden="true" />
          </Button>
        </div>
      {/if}

      <div class="flex items-center gap-0.5">
        {#if panel.file.staged}
          <Button
            class="h-6 px-1.5 text-xs"
            disabled={panel.runningAction}
            size="sm"
            variant="outline"
            onclick={unstageSelectedFile}
          >
            <MinusIcon class="size-3" aria-hidden="true" />
            <span>{m.git_changes_unstage_file()}</span>
          </Button>
        {:else}
          <Button
            class="h-6 px-1.5 text-xs"
            disabled={panel.runningAction}
            size="sm"
            variant="outline"
            onclick={stageSelectedFile}
          >
            <PlusIcon class="size-3" aria-hidden="true" />
            <span>{m.git_changes_stage_file()}</span>
          </Button>

          <Button
            class="size-6 text-destructive hover:text-destructive"
            aria-label={isUntracked ? m.git_changes_delete_untracked() : m.git_changes_discard_file()}
            disabled={panel.runningAction}
            size="icon"
            title={isUntracked ? m.git_changes_delete_untracked() : m.git_changes_discard_file()}
            variant="ghost"
            onclick={discardSelectedFile}
          >
            {#if isUntracked}
              <TrashIcon class="size-3.5" aria-hidden="true" />
            {:else}
              <Undo2Icon class="size-3.5" aria-hidden="true" />
            {/if}
          </Button>
        {/if}

        <Button
          class="size-6"
          aria-label={panel.diffMode === 'unified' ? m.git_diff_split() : m.git_diff_unified()}
          aria-pressed={panel.diffMode === 'split'}
          size="icon"
          title={panel.diffMode === 'unified' ? m.git_diff_split() : m.git_diff_unified()}
          variant={panel.diffMode === 'split' ? 'secondary' : 'ghost'}
          onclick={toggleDiffMode}
        >
          {#if panel.diffMode === 'unified'}
            <Columns2Icon class="size-3.5" aria-hidden="true" />
          {:else}
            <Rows2Icon class="size-3.5" aria-hidden="true" />
          {/if}
        </Button>

        {#if !isUntracked}
          <Button
            class="size-6"
            aria-label={m.git_diff_ignore_whitespace()}
            aria-pressed={panel.ignoreWhitespace}
            size="icon"
            title={m.git_diff_ignore_whitespace()}
            variant={panel.ignoreWhitespace ? 'secondary' : 'ghost'}
            onclick={() => panel.toggleIgnoreWhitespace()}
          >
            <SpaceIcon class="size-3.5" aria-hidden="true" />
          </Button>

          <Button
            class="size-6"
            aria-label={m.git_changes_open_blame()}
            size="icon"
            title={m.git_changes_open_blame()}
            variant="ghost"
            onclick={openBlame}
          >
            <ScanSearchIcon class="size-3.5" aria-hidden="true" />
          </Button>

          <Button
            class="size-6"
            aria-label={m.git_changes_open_history()}
            size="icon"
            title={m.git_changes_open_history()}
            variant="ghost"
            onclick={openHistory}
          >
            <FileClockIcon class="size-3.5" aria-hidden="true" />
          </Button>
        {/if}

        <Button
          class="size-6"
          aria-label={copiedPath ? m.git_diff_copied() : m.git_diff_copy_path()}
          size="icon"
          title={copiedPath ? m.git_diff_copied() : m.git_diff_copy_path()}
          variant="ghost"
          onclick={copyPath}
        >
          {#if copiedPath}
            <CheckIcon class="size-3.5 text-git-added" aria-hidden="true" />
          {:else}
            <CopyIcon class="size-3.5" aria-hidden="true" />
          {/if}
        </Button>
      </div>
    </div>

    <div class="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto p-2.5">
      {#if isLoadingInitial}
        <div class="flex items-center gap-2 py-4 text-xs text-muted-foreground" aria-busy="true">
          <Spinner aria-label={m.git_panel_loading()} />
          <span>{m.git_panel_loading()}</span>
        </div>
      {:else if loadFailure !== null}
        <Alert.Root variant="destructive">
          <Alert.Description class="flex items-center justify-between gap-2 text-xs">
            <span>{gitFailureMessage(loadFailure)}</span>
            <Button size="sm" variant="outline" onclick={() => panel.readFile()}>{m.git_panel_try_again()}</Button>
          </Alert.Description>
        </Alert.Root>
      {:else if diff !== null}
        {#if diff.truncated}
          <p class="rounded-md border bg-muted/40 px-2.5 py-1.5 text-xs text-muted-foreground">
            {m.git_changes_truncated_notice()}
          </p>
        {/if}

        {#if diff.binary}
          <p class="py-4 text-center text-xs text-muted-foreground">{m.git_diff_binary()}</p>
        {:else if diff.hunks.length === 0}
          <p class="py-4 text-center text-xs text-muted-foreground">{m.git_diff_no_changes()}</p>
        {:else}
          <div class="flex flex-col gap-2">
            {#each diff.hunks as hunk, index (index)}
              <div class="overflow-hidden rounded-md border">
                <HunkLines
                  busy={panel.runningAction}
                  {hunk}
                  mode={panel.diffMode}
                  onDiscard={canStageOrDiscardHunk ? () => discardHunk(hunk) : undefined}
                  onStage={canStageOrDiscardHunk ? () => stageHunk(hunk) : undefined}
                  onUnstage={canUnstageHunk ? () => unstageHunk(hunk) : undefined}
                />
              </div>
            {/each}
          </div>
        {/if}
      {/if}
    </div>
  </div>
{/if}
