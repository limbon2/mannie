<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import FileClockIcon from '@mannie/plugin-sdk/icons/file-clock';
  import MinusIcon from '@mannie/plugin-sdk/icons/minus';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';
  import ScanSearchIcon from '@mannie/plugin-sdk/icons/scan-search';
  import TrashIcon from '@mannie/plugin-sdk/icons/trash';
  import Undo2Icon from '@mannie/plugin-sdk/icons/undo-2';

  import type { Entry, FileStat } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';
  import type { RepositoryPanel } from '../../state/git-repository.svelte.js';
  import type { ChangeGroupKind } from '../../types/git.js';
  import { changeWords, conflictWords } from '../../utils/git-messages.js';
  import { splitPath } from '../../utils/git-tree.js';

  // One file row inside the Changes list or compressed tree: status letter,
  // filename, optional directory/rename suffix, inline `+N -M` line counts,
  // and hover/focus stage, unstage, discard, blame and file-history controls.
  interface Props {
    readonly entry: Entry;
    readonly groupKind: ChangeGroupKind;
    readonly panel: RepositoryPanel;
    readonly showDirectory: boolean;
    readonly stat: FileStat | undefined;
  }

  let { entry, groupKind, panel, showDirectory, stat }: Props = $props();

  const parts = $derived(splitPath(entry.path));
  const selected = $derived.by(() => {
    if (panel.file === null) {
      return false;
    }
    const isSamePath = panel.file.path === entry.path;
    const isSameStage = panel.file.staged === (groupKind === 'staged');
    return isSamePath && isSameStage;
  });
  const showDirText = $derived(showDirectory && parts.directory.length > 0);
  const numericStat = $derived.by(() => {
    if (stat === undefined) {
      return null;
    }
    const { added, removed } = stat;
    if (added === null) {
      return null;
    }
    if (removed === null) {
      return null;
    }
    return { added, removed };
  });
  const letter = $derived.by(() => {
    if (groupKind === 'conflicted') {
      return m.git_status_letter_conflicted();
    }
    if (groupKind === 'untracked') {
      return m.git_status_letter_untracked();
    }
    const raw = groupKind === 'staged' ? entry.staged : entry.unstaged;
    switch (raw) {
      case 'A': {
        return m.git_status_letter_added();
      }
      case 'C': {
        return m.git_status_letter_copied();
      }
      case 'D': {
        return m.git_status_letter_deleted();
      }
      case 'M': {
        return m.git_status_letter_modified();
      }
      case 'R': {
        return m.git_status_letter_renamed();
      }
      default: {
        return raw;
      }
    }
  });
  const colorClass = $derived.by(() => {
    if (groupKind === 'conflicted') {
      return 'text-git-conflicted';
    }
    if (groupKind === 'untracked') {
      return 'text-git-untracked';
    }
    const raw = groupKind === 'staged' ? entry.staged : entry.unstaged;
    switch (raw) {
      case 'A': {
        return 'text-git-added';
      }
      case 'D': {
        return 'text-git-deleted';
      }
      case 'R':
      case 'C': {
        return 'text-git-renamed';
      }
      default: {
        return 'text-git-modified';
      }
    }
  });
  const spoken = $derived.by(() => {
    if (entry.kind === 'conflicted') {
      return conflictWords(entry.conflict);
    }
    if (groupKind === 'untracked') {
      return m.git_status_untracked();
    }
    return changeWords(groupKind === 'staged' ? entry.staged : entry.unstaged);
  });

  function selectEntry(): void {
    if (groupKind === 'conflicted') {
      panel.activeTab = 'conflicts';
      void panel.readConflicts();
      return;
    }
    panel.chooseFile(entry.path, groupKind === 'staged');
  }

  function stageEntry(event: MouseEvent): void {
    event.stopPropagation();
    void panel.runAction({ all: false, kind: 'stage_files', paths: [entry.path] });
  }

  function unstageEntry(event: MouseEvent): void {
    event.stopPropagation();
    void panel.runAction({ all: false, kind: 'unstage_files', paths: [entry.path] });
  }

  function discardEntry(event: MouseEvent): void {
    event.stopPropagation();
    void panel.runAction({ includeUntracked: groupKind === 'untracked', kind: 'discard_files', paths: [entry.path] });
  }

  function openBlame(event: MouseEvent): void {
    event.stopPropagation();
    panel.openFileBlame(entry.path);
  }

  function openHistory(event: MouseEvent): void {
    event.stopPropagation();
    panel.openFileHistory(entry.path);
  }
</script>

<div
  class={[
    'group/row flex min-w-0 items-center gap-1 rounded-sm pe-1',
    selected ? 'bg-accent text-accent-foreground' : 'hover:bg-muted/70',
  ]}
>
  <button
    class="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5 px-1.5 py-1 text-start text-xs"
    aria-current={selected ? 'true' : undefined}
    data-git-entry-group={groupKind}
    data-git-entry-path={entry.path}
    title={entry.path}
    type="button"
    onclick={selectEntry}
  >
    <span class={['w-3.5 shrink-0 text-center font-mono text-[11px] font-semibold', colorClass]} aria-hidden="true">
      {letter}
    </span>
    <span class="sr-only">{spoken}</span>

    <span class="min-w-0 truncate font-medium text-foreground">{parts.fileName}</span>

    {#if showDirText}
      <span class="min-w-0 truncate text-muted-foreground">{parts.directory}</span>
    {/if}

    {#if entry.kind === 'renamed'}
      <span class="truncate text-muted-foreground">{m.git_renamed_from({ path: entry.original })}</span>
    {/if}

    {#if entry.kind === 'conflicted'}
      <span class="ms-auto shrink-0 text-[11px] text-git-conflicted">{spoken}</span>
    {:else if numericStat !== null}
      <span class="ms-auto flex shrink-0 gap-1 font-mono text-[11px] tabular-nums">
        {#if numericStat.added > 0}
          <span class="text-diff-added-foreground">{m.git_diff_added({ count: formatCount(numericStat.added) })}</span>
        {/if}
        {#if numericStat.removed > 0}
          <span class="text-diff-removed-foreground">
            {m.git_diff_removed({ count: formatCount(numericStat.removed) })}
          </span>
        {/if}
      </span>
    {/if}
  </button>

  {#if groupKind !== 'conflicted'}
    <div
      class="flex shrink-0 items-center gap-0.5 opacity-0 transition-opacity group-focus-within/row:opacity-100 group-hover/row:opacity-100"
    >
      {#if groupKind === 'staged'}
        <Button
          class="size-5"
          aria-label={m.git_changes_unstage_file()}
          disabled={panel.runningAction}
          size="icon"
          title={m.git_changes_unstage_file()}
          variant="ghost"
          onclick={unstageEntry}
        >
          <MinusIcon class="size-3" aria-hidden="true" />
        </Button>
      {:else}
        <Button
          class="size-5"
          aria-label={m.git_changes_stage_file()}
          disabled={panel.runningAction}
          size="icon"
          title={m.git_changes_stage_file()}
          variant="ghost"
          onclick={stageEntry}
        >
          <PlusIcon class="size-3" aria-hidden="true" />
        </Button>

        <Button
          class="size-5 text-destructive hover:text-destructive"
          aria-label={groupKind === 'untracked' ? m.git_changes_delete_untracked() : m.git_changes_discard_file()}
          disabled={panel.runningAction}
          size="icon"
          title={groupKind === 'untracked' ? m.git_changes_delete_untracked() : m.git_changes_discard_file()}
          variant="ghost"
          onclick={discardEntry}
        >
          {#if groupKind === 'untracked'}
            <TrashIcon class="size-3" aria-hidden="true" />
          {:else}
            <Undo2Icon class="size-3" aria-hidden="true" />
          {/if}
        </Button>
      {/if}

      {#if groupKind !== 'untracked'}
        <Button
          class="size-5"
          aria-label={m.git_changes_open_blame()}
          size="icon"
          title={m.git_changes_open_blame()}
          variant="ghost"
          onclick={openBlame}
        >
          <ScanSearchIcon class="size-3" aria-hidden="true" />
        </Button>

        <Button
          class="size-5"
          aria-label={m.git_changes_open_history()}
          size="icon"
          title={m.git_changes_open_history()}
          variant="ghost"
          onclick={openHistory}
        >
          <FileClockIcon class="size-3" aria-hidden="true" />
        </Button>
      {/if}
    </div>
  {/if}
</div>
