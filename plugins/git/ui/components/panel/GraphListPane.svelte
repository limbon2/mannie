<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { Checkbox } from '@mannie/plugin-sdk/components/checkbox';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import * as Select from '@mannie/plugin-sdk/components/select';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import FunnelIcon from '@mannie/plugin-sdk/icons/funnel';
  import SearchIcon from '@mannie/plugin-sdk/icons/search';
  import XIcon from '@mannie/plugin-sdk/icons/x';

  import { m } from '../../generated/messages.js';
  import type { RepositoryPanel } from '../../state/git-repository.svelte.js';
  import { formatFullDate, formatRelativeTime } from '../../utils/git-format.js';
  import { layoutCommitGraph } from '../../utils/git-graph.js';
  import { gitFailureMessage } from '../../utils/git-messages.js';
  import RefBadge from '../cards/RefBadge.svelte';

  import CommitGraphSvg from './CommitGraphSvg.svelte';

  // Topological commit graph list with scope selector, search and filter bar,
  // SVG branch/merge lanes, non-clipping ref badges, and pagination.
  interface Props {
    readonly panel: RepositoryPanel;
  }

  let { panel }: Props = $props();

  let showFilters = $state(false);

  const scopeValue = $derived(panel.historyScope.kind);
  const isReflog = $derived(panel.historyScope.kind === 'reflog');
  const graphRows = $derived(isReflog ? [] : layoutCommitGraph(panel.graphCommits));
  const activeFilterCount = $derived(
    (panel.historyAuthor.trim().length > 0 ? 1 : 0) +
      (panel.historyPath.trim().length > 0 ? 1 : 0) +
      (panel.historyPickaxe.trim().length > 0 ? 1 : 0) +
      (panel.historyMerges !== 'all' ? 1 : 0) +
      (panel.historyFirstParent ? 1 : 0),
  );
  const isFilterActive = $derived(showFilters || activeFilterCount > 0);
  const isLoadingInitial = $derived.by(() => {
    const isLoading = panel.graphPage.status === 'loading';
    const hasNoCommits = panel.graphCommits.length === 0;
    const hasNoReflog = panel.graphReflog.length === 0;
    return isLoading && hasNoCommits && hasNoReflog;
  });
  const loadFailure = $derived(panel.graphPage.status === 'error' ? panel.graphPage.failure : null);
  const hasSearchOrFilters = $derived.by(() => {
    const hasFilters = activeFilterCount > 0;
    const hasQuery = panel.historyQuery.trim().length > 0;
    return hasFilters || hasQuery;
  });

  function handleScopeChange(next: string): void {
    if (next === 'all_branches') {
      panel.historyScope = { kind: 'all_branches' };
    } else if (next === 'reflog') {
      panel.historyScope = { kind: 'reflog' };
    } else {
      panel.historyScope = { kind: 'current_branch' };
    }
    void panel.readGraph(true);
  }

  function handleMergeFilterChange(next: string): void {
    const isOnly = next === 'only';
    const isExclude = next === 'exclude';
    const isAll = next === 'all';
    if (isOnly || isExclude || isAll) {
      panel.historyMerges = next;
      void panel.readGraph(true);
    }
  }

  function submitFilters(event: SubmitEvent): void {
    event.preventDefault();
    void panel.readGraph(true);
  }

  function toggleFilters(): void {
    showFilters = !showFilters;
  }

  function toggleCommit(oid: string): void {
    if (panel.chosenCommit === oid) {
      panel.closeCommit();
    } else {
      panel.chooseCommit(oid);
    }
  }

  function handleFirstParentChange(checked: boolean): void {
    panel.historyFirstParent = checked;
    void panel.readGraph(true);
  }
</script>

<div class="flex h-full min-h-0 flex-col overflow-hidden">
  <form class="flex flex-col gap-1.5 border-b px-2.5 py-2" onsubmit={submitFilters}>
    <div class="flex items-center gap-1.5">
      <Select.Root type="single" value={scopeValue} onValueChange={handleScopeChange}>
        <Select.Trigger class="h-7 w-36 shrink-0 text-xs">
          {#if scopeValue === 'all_branches'}
            {m.git_graph_scope_all()}
          {:else if scopeValue === 'reflog'}
            {m.git_graph_scope_reflog()}
          {:else}
            {m.git_graph_scope_current()}
          {/if}
        </Select.Trigger>
        <Select.Content>
          <Select.Item value="current_branch">{m.git_graph_scope_current()}</Select.Item>
          <Select.Item value="all_branches">{m.git_graph_scope_all()}</Select.Item>
          <Select.Item value="reflog">{m.git_graph_scope_reflog()}</Select.Item>
        </Select.Content>
      </Select.Root>

      {#if !isReflog}
        <div class="relative min-w-0 flex-1">
          <SearchIcon
            class="pointer-events-none absolute inset-s-2 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground"
            aria-hidden="true"
          />
          <Input
            class="h-7 ps-7 text-xs"
            aria-label={m.git_graph_search_placeholder()}
            placeholder={m.git_graph_search_placeholder()}
            bind:value={panel.historyQuery}
          />
        </div>

        <Button
          class="h-7 gap-1 px-2 text-xs"
          aria-expanded={showFilters}
          size="sm"
          variant={isFilterActive ? 'secondary' : 'ghost'}
          onclick={toggleFilters}
        >
          <FunnelIcon class="size-3.5" aria-hidden="true" />
          <span>{m.git_graph_filters_toggle()}</span>
          {#if activeFilterCount > 0}
            <Badge class="h-4 px-1 text-[10px]" variant="default">{activeFilterCount}</Badge>
          {/if}
        </Button>
      {/if}
    </div>

    {#if showFilters && !isReflog}
      <div class="grid grid-cols-1 gap-1.5 rounded-md border bg-muted/30 p-2 sm:grid-cols-2">
        <Input
          class="h-7 text-xs"
          aria-label={m.git_graph_filter_author()}
          placeholder={m.git_graph_filter_author()}
          bind:value={panel.historyAuthor}
        />
        <Input
          class="h-7 text-xs"
          aria-label={m.git_graph_filter_path()}
          placeholder={m.git_graph_filter_path()}
          bind:value={panel.historyPath}
        />
        <Input
          class="h-7 text-xs"
          aria-label={m.git_graph_filter_pickaxe()}
          placeholder={m.git_graph_filter_pickaxe()}
          bind:value={panel.historyPickaxe}
        />
        <Select.Root type="single" value={panel.historyMerges} onValueChange={handleMergeFilterChange}>
          <Select.Trigger class="h-7 text-xs">
            {#if panel.historyMerges === 'only'}
              {m.git_graph_filter_merges_only()}
            {:else if panel.historyMerges === 'exclude'}
              {m.git_graph_filter_merges_exclude()}
            {:else}
              {m.git_graph_filter_merges_all()}
            {/if}
          </Select.Trigger>
          <Select.Content>
            <Select.Item value="all">{m.git_graph_filter_merges_all()}</Select.Item>
            <Select.Item value="only">{m.git_graph_filter_merges_only()}</Select.Item>
            <Select.Item value="exclude">{m.git_graph_filter_merges_exclude()}</Select.Item>
          </Select.Content>
        </Select.Root>

        <div class="flex items-center justify-between gap-2 sm:col-span-2">
          <label class="flex cursor-pointer items-center gap-1.5 text-xs text-muted-foreground">
            <Checkbox checked={panel.historyFirstParent} onCheckedChange={handleFirstParentChange} />
            <span>{m.git_graph_filter_first_parent()}</span>
          </label>

          <div class="flex items-center gap-1">
            <Button
              class="h-6 px-2 text-xs"
              size="sm"
              type="button"
              variant="ghost"
              onclick={() => panel.clearGraphFilters()}
            >
              <XIcon class="size-3" aria-hidden="true" />
              <span>{m.git_graph_clear_filters()}</span>
            </Button>
            <Button class="h-6 px-2 text-xs" size="sm" type="submit">{m.git_inspect_grep_submit()}</Button>
          </div>
        </div>
      </div>
    {/if}
  </form>

  <div class="min-h-0 flex-1 overflow-y-auto" data-git-commits="">
    {#if isLoadingInitial}
      <div class="flex items-center gap-2 p-3 text-xs text-muted-foreground" aria-busy="true">
        <Spinner aria-label={m.git_panel_loading()} />
        <span>{m.git_panel_loading()}</span>
      </div>
    {:else if loadFailure !== null}
      <div class="p-3">
        <Alert.Root variant="destructive">
          <Alert.Description class="flex items-center justify-between gap-2 text-xs">
            <span>{gitFailureMessage(loadFailure)}</span>
            <Button size="sm" variant="outline" onclick={() => panel.readGraph(true)}>{m.git_panel_try_again()}</Button>
          </Alert.Description>
        </Alert.Root>
      </div>
    {:else if isReflog}
      {#if panel.graphReflog.length === 0}
        <p class="p-4 text-center text-xs text-muted-foreground">{m.git_graph_reflog_empty()}</p>
      {:else}
        <ul class="flex flex-col divide-y">
          {#each panel.graphReflog as entry (`${entry.selector}-${entry.oid}`)}
            {const selected = $derived(panel.chosenCommit === entry.oid)}
            <li>
              <button
                class={[
                  'flex w-full cursor-pointer items-center gap-2 px-2.5 py-1.5 text-start text-xs hover:bg-muted/70',
                  selected ? 'bg-accent text-accent-foreground' : '',
                ]}
                aria-current={selected ? 'true' : undefined}
                type="button"
                onclick={() => toggleCommit(entry.oid)}
              >
                <Badge class="shrink-0 font-mono text-[10px]" variant="outline">{entry.selector}</Badge>
                <span class="shrink-0 font-mono text-muted-foreground">{entry.short}</span>
                <span class="min-w-0 flex-1 truncate">{entry.action}</span>
                <span class="shrink-0 text-muted-foreground tabular-nums" title={formatFullDate(entry.committed)}>
                  {formatRelativeTime(entry.committed)}
                </span>
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    {:else if panel.graphCommits.length === 0}
      <p class="p-4 text-center text-xs text-muted-foreground">
        {hasSearchOrFilters ? m.git_graph_no_matches() : m.git_panel_history_empty()}
      </p>
    {:else}
      <ul class="flex flex-col">
        {#each panel.graphCommits as commitItem, idx (commitItem.oid)}
          {const row = $derived(graphRows.at(idx))}
          {const selected = $derived(panel.chosenCommit === commitItem.oid)}
          {const isHead = $derived(panel.graphHeadOid === commitItem.oid)}
          <li>
            <button
              class={[
                'flex h-7 w-full cursor-pointer items-center gap-1.5 ps-1 pe-2.5 text-start text-xs hover:bg-muted/70',
                selected ? 'bg-accent text-accent-foreground' : '',
              ]}
              aria-current={selected ? 'true' : undefined}
              data-git-commit-oid={commitItem.oid}
              type="button"
              onclick={() => toggleCommit(commitItem.oid)}
            >
              {#if row !== undefined}
                <CommitGraphSvg {isHead} {row} />
              {/if}
              <span class="shrink-0 font-mono text-[11px] text-muted-foreground">{commitItem.short}</span>
              <span class="min-w-0 flex-1 truncate font-medium">{commitItem.subject}</span>
              {#each commitItem.refs as reference (reference)}
                <RefBadge {reference} />
              {/each}
              <span class="hidden max-w-24 shrink-0 truncate text-[11px] text-muted-foreground sm:inline">
                {commitItem.author}
              </span>
              {#if commitItem.authored.length > 0}
                <span
                  class="shrink-0 text-[11px] text-muted-foreground tabular-nums"
                  title={formatFullDate(commitItem.authored)}
                >
                  {formatRelativeTime(commitItem.authored)}
                </span>
              {/if}
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    {#if panel.graphMore}
      <div class="flex justify-center border-t p-2">
        <Button
          disabled={panel.graphPage.status === 'loading'}
          size="sm"
          variant="outline"
          onclick={() => panel.loadMoreGraph()}
        >
          {m.git_panel_show_more()}
        </Button>
      </div>
    {/if}
  </div>
</div>
