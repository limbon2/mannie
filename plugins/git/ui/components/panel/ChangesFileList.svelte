<script lang="ts">
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Collapsible from '@mannie/plugin-sdk/components/collapsible';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import ChevronRightIcon from '@mannie/plugin-sdk/icons/chevron-right';
  import FolderTreeIcon from '@mannie/plugin-sdk/icons/folder-tree';
  import ListIcon from '@mannie/plugin-sdk/icons/list';
  import MinusIcon from '@mannie/plugin-sdk/icons/minus';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';
  import SearchIcon from '@mannie/plugin-sdk/icons/search';

  import type { Entry, FileStat, RepositoryOverview } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';
  import type { RepositoryPanel } from '../../state/git-repository.svelte.js';
  import type { ChangeGroupKind } from '../../types/git.js';
  import { buildFileTree } from '../../utils/git-tree.js';

  import ChangesFileRow from './ChangesFileRow.svelte';
  import ChangesFileTree from './ChangesFileTree.svelte';

  // Filterable, tree/flat toggleable list of conflicted, staged, unstaged
  // and untracked files with group-level stage/unstage controls.
  interface Props {
    readonly overview: RepositoryOverview;
    readonly panel: RepositoryPanel;
  }

  interface FileGroup {
    readonly entries: readonly Entry[];
    readonly heading: string;
    readonly kind: ChangeGroupKind;
  }

  let { overview, panel }: Props = $props();

  let filterText = $state('');

  const stagedStatMap = $derived(new Map(overview.stagedStats.map((stat) => [stat.path, stat])));
  const unstagedStatMap = $derived(new Map(overview.unstagedStats.map((stat) => [stat.path, stat])));
  const needle = $derived(filterText.trim().toLowerCase());
  const leftOut = $derived(Math.max(0, overview.entriesTotal - overview.entries.length));
  const groups: readonly FileGroup[] = $derived.by(() => {
    const filtered = overview.entries.filter(matchesFilter);
    const conflicted = filtered.filter((entry) => entry.kind === 'conflicted');
    const staged = filtered.filter(isStagedEntry);
    const unstaged = filtered.filter(isUnstagedEntry);
    const untracked = filtered.filter((entry) => entry.kind === 'untracked');

    const result: FileGroup[] = [];
    if (conflicted.length > 0) {
      result.push({ entries: conflicted, heading: m.git_status_conflicted(), kind: 'conflicted' });
    }
    if (staged.length > 0) {
      result.push({ entries: staged, heading: m.git_status_staged(), kind: 'staged' });
    }
    if (unstaged.length > 0) {
      result.push({ entries: unstaged, heading: m.git_status_unstaged(), kind: 'unstaged' });
    }
    if (untracked.length > 0) {
      result.push({ entries: untracked, heading: m.git_status_untracked(), kind: 'untracked' });
    }
    return result;
  });

  function isTrackedEntry(entry: Entry): boolean {
    const isConflicted = entry.kind === 'conflicted';
    const isUntracked = entry.kind === 'untracked';
    const isIgnored = entry.kind === 'ignored';
    return !isConflicted && !isUntracked && !isIgnored;
  }

  function isStagedEntry(entry: Entry): boolean {
    const hasIndexChange = entry.staged !== '.';
    return isTrackedEntry(entry) && hasIndexChange;
  }

  function isUnstagedEntry(entry: Entry): boolean {
    const hasWorktreeChange = entry.unstaged !== '.';
    return isTrackedEntry(entry) && hasWorktreeChange;
  }

  function matchesFilter(entry: Entry): boolean {
    const isUnfiltered = needle.length === 0;
    const hasMatch = entry.path.toLowerCase().includes(needle);
    return isUnfiltered || hasMatch;
  }

  function statFor(path: string, groupKind: ChangeGroupKind): FileStat | undefined {
    if (groupKind === 'staged') {
      return stagedStatMap.get(path);
    }
    if (groupKind === 'unstaged') {
      return unstagedStatMap.get(path);
    }
    return undefined;
  }

  function isStageableGroup(kind: ChangeGroupKind): boolean {
    const isUnstaged = kind === 'unstaged';
    const isUntracked = kind === 'untracked';
    return isUnstaged || isUntracked;
  }

  function toggleViewMode(): void {
    panel.fileViewMode = panel.fileViewMode === 'tree' ? 'list' : 'tree';
  }

  function unstageGroup(group: FileGroup, event: MouseEvent): void {
    event.stopPropagation();
    void panel.runAction({
      all: false,
      kind: 'unstage_files',
      paths: group.entries.map((entry) => entry.path),
    });
  }

  function stageGroup(group: FileGroup, event: MouseEvent): void {
    event.stopPropagation();
    void panel.runAction({
      all: false,
      kind: 'stage_files',
      paths: group.entries.map((entry) => entry.path),
    });
  }
</script>

<div class="flex min-h-0 flex-1 flex-col overflow-hidden" data-git-entries="">
  <div class="flex items-center gap-1.5 border-b px-2.5 py-1.5">
    <div class="relative flex-1">
      <SearchIcon
        class="pointer-events-none absolute inset-s-2 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground"
        aria-hidden="true"
      />
      <Input
        class="h-7 ps-7 text-xs"
        aria-label={m.git_changes_filter_placeholder()}
        placeholder={m.git_changes_filter_placeholder()}
        bind:value={filterText}
      />
    </div>

    <Button
      class="size-7 shrink-0"
      aria-label={panel.fileViewMode === 'tree' ? m.git_changes_view_list() : m.git_changes_view_tree()}
      size="icon"
      title={panel.fileViewMode === 'tree' ? m.git_changes_view_list() : m.git_changes_view_tree()}
      variant="ghost"
      onclick={toggleViewMode}
    >
      {#if panel.fileViewMode === 'tree'}
        <ListIcon class="size-3.5" aria-hidden="true" />
      {:else}
        <FolderTreeIcon class="size-3.5" aria-hidden="true" />
      {/if}
    </Button>
  </div>

  <div class="flex min-h-0 flex-1 flex-col gap-2 overflow-y-auto p-2">
    {#if overview.entries.length === 0}
      <p class="px-1 py-4 text-center text-xs text-muted-foreground">{m.git_status_clean()}</p>
    {:else if groups.length === 0}
      <p class="px-1 py-4 text-center text-xs text-muted-foreground">{m.git_refs_empty()}</p>
    {:else}
      {#each groups as group (group.kind)}
        <Collapsible.Root class="group/section flex flex-col gap-1" open={true}>
          <div class="flex items-center justify-between gap-1 px-1">
            <Collapsible.Trigger
              class="flex min-w-0 items-center gap-1.5 text-xs font-medium text-muted-foreground hover:text-foreground"
            >
              <ChevronRightIcon
                class="size-3.5 shrink-0 transition-transform group-data-[state=open]/section:rotate-90"
                aria-hidden="true"
              />
              <span>{group.heading}</span>
              <Badge class="h-4 px-1.5 text-[10px]" variant="secondary">{formatCount(group.entries.length)}</Badge>
            </Collapsible.Trigger>

            {#if group.kind === 'staged'}
              <Button
                class="h-5 px-1.5 text-[11px]"
                disabled={panel.runningAction}
                size="sm"
                title={m.git_changes_unstage_all()}
                variant="ghost"
                onclick={(event: MouseEvent) => unstageGroup(group, event)}
              >
                <MinusIcon class="size-3" aria-hidden="true" />
                <span>{m.git_changes_unstage_all()}</span>
              </Button>
            {:else if isStageableGroup(group.kind)}
              <Button
                class="h-5 px-1.5 text-[11px]"
                disabled={panel.runningAction}
                size="sm"
                title={m.git_changes_stage_all()}
                variant="ghost"
                onclick={(event: MouseEvent) => stageGroup(group, event)}
              >
                <PlusIcon class="size-3" aria-hidden="true" />
                <span>{m.git_changes_stage_all()}</span>
              </Button>
            {/if}
          </div>

          <Collapsible.Content>
            {#if panel.fileViewMode === 'tree'}
              <ChangesFileTree
                groupKind={group.kind}
                nodes={buildFileTree(group.entries, (entry) => entry.path)}
                {panel}
                {statFor}
              />
            {:else}
              <ul class="flex flex-col gap-0.5">
                {#each group.entries as entry (`${group.kind}-${entry.path}`)}
                  <li>
                    <ChangesFileRow
                      {entry}
                      groupKind={group.kind}
                      {panel}
                      showDirectory={true}
                      stat={statFor(entry.path, group.kind)}
                    />
                  </li>
                {/each}
              </ul>
            {/if}
          </Collapsible.Content>
        </Collapsible.Root>
      {/each}

      {#if leftOut > 0}
        <p class="px-1 text-xs text-muted-foreground">{m.git_status_more({ count: formatCount(leftOut) })}</p>
      {/if}
    {/if}
  </div>
</div>
