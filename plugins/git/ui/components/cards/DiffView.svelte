<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Collapsible from '@mannie/plugin-sdk/components/collapsible';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import ChevronRightIcon from '@mannie/plugin-sdk/icons/chevron-right';
  import FileClockIcon from '@mannie/plugin-sdk/icons/file-clock';
  import ScanSearchIcon from '@mannie/plugin-sdk/icons/scan-search';

  import type { FilePatch, FileStat, Hunk } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';

  import HunkLines from './HunkLines.svelte';

  // A diff (ADR-0039; plugins/git/docs/panel.md): each changed file with its
  // line counts, which opens to its patch. Supports unified/split modes,
  // per-hunk actions and quick jumps to Blame/File History when used inside
  // the side panel while staying compact in transcript cards.
  interface Props {
    readonly busy?: boolean;
    readonly files: readonly FileStat[];
    readonly mode?: 'split' | 'unified';
    readonly onDiscardHunk?: (path: string, hunk: Hunk, original: string | null) => void;
    readonly onOpenBlame?: (path: string) => void;
    readonly onOpenHistory?: (path: string) => void;
    readonly onStageHunk?: (path: string, hunk: Hunk, original: string | null) => void;
    readonly onUnstageHunk?: (path: string, hunk: Hunk, original: string | null) => void;
    /** Whether every file starts open, as for a single file. */
    readonly open?: boolean;
    readonly patches: readonly FilePatch[];
  }

  interface LineCounts {
    readonly added: number;
    readonly removed: number;
  }

  let {
    busy = false,
    files,
    mode = 'unified',
    onDiscardHunk,
    onOpenBlame,
    onOpenHistory,
    onStageHunk,
    onUnstageHunk,
    open = false,
    patches,
  }: Props = $props();

  const unpatched = $derived(
    patches.length === 0 ? 0 : files.filter((file) => patchOf(file.path) === undefined).length,
  );

  /** A file's counts of added and removed lines, or `null` for a binary file, which has none. */
  function lineCounts(file: FileStat): LineCounts | null {
    const { added, removed } = file;
    if (added === null) {
      return null;
    }
    return removed === null ? null : { added, removed };
  }

  function patchOf(path: string): FilePatch | undefined {
    return patches.find((patch) => patch.path === path);
  }
</script>

<div class="flex flex-col gap-1.5" data-git-diff="">
  {#each files as file (file.path)}
    {const patch = $derived(patchOf(file.path))}
    {const counts = $derived(lineCounts(file))}
    <Collapsible.Root class="group/file rounded-md border" {open}>
      <div class="flex min-w-0 items-center gap-1 pe-1">
        <Collapsible.Trigger
          class="flex min-w-0 flex-1 items-center gap-2 px-2 py-1 text-start text-sm hover:bg-muted"
          disabled={patch === undefined}
        >
          <ChevronRightIcon
            class="size-4 shrink-0 transition-transform group-data-[state=open]/file:rotate-90"
            aria-hidden="true"
          />
          <span class="min-w-0 truncate font-mono" title={file.path}>{file.path}</span>
          {#if file.original !== null}
            <span class="truncate text-muted-foreground">{m.git_renamed_from({ path: file.original })}</span>
          {/if}
          {#if counts === null}
            <span class="ms-auto shrink-0 text-muted-foreground">{m.git_diff_binary()}</span>
          {:else}
            <span class="ms-auto flex shrink-0 gap-1 tabular-nums" data-git-diff-stat="">
              <span class="text-diff-added-foreground">{m.git_diff_added({ count: formatCount(counts.added) })}</span>
              <span class="text-diff-removed-foreground">
                {m.git_diff_removed({ count: formatCount(counts.removed) })}
              </span>
            </span>
          {/if}
        </Collapsible.Trigger>

        {#if onOpenBlame !== undefined}
          <Button
            class="size-6 shrink-0"
            aria-label={m.git_changes_open_blame()}
            size="icon"
            title={m.git_changes_open_blame()}
            variant="ghost"
            onclick={() => onOpenBlame(file.path)}
          >
            <ScanSearchIcon class="size-3.5" aria-hidden="true" />
          </Button>
        {/if}

        {#if onOpenHistory !== undefined}
          <Button
            class="size-6 shrink-0"
            aria-label={m.git_changes_open_history()}
            size="icon"
            title={m.git_changes_open_history()}
            variant="ghost"
            onclick={() => onOpenHistory(file.path)}
          >
            <FileClockIcon class="size-3.5" aria-hidden="true" />
          </Button>
        {/if}
      </div>

      {#if patch !== undefined}
        <Collapsible.Content class="border-t">
          {#if patch.hunks.length === 0}
            <p class="px-2 py-1 text-sm text-muted-foreground">
              {file.added === null ? m.git_diff_binary() : m.git_diff_no_changes()}
            </p>
          {:else}
            {#each patch.hunks as hunk, index (index)}
              <HunkLines
                {busy}
                {hunk}
                {mode}
                onDiscard={onDiscardHunk === undefined
                  ? undefined
                  : () => onDiscardHunk(file.path, hunk, file.original)}
                onStage={onStageHunk === undefined ? undefined : () => onStageHunk(file.path, hunk, file.original)}
                onUnstage={onUnstageHunk === undefined
                  ? undefined
                  : () => onUnstageHunk(file.path, hunk, file.original)}
              />
            {/each}
          {/if}
        </Collapsible.Content>
      {/if}
    </Collapsible.Root>
  {/each}

  {#if unpatched > 0}
    <p class="text-sm text-muted-foreground">{m.git_diff_patches_left({ count: unpatched })}</p>
  {/if}
</div>
