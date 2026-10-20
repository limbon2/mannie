<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import { Skeleton } from '@mannie/plugin-sdk/components/skeleton';
  import * as Tabs from '@mannie/plugin-sdk/components/tabs';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import ArrowLeftIcon from '@mannie/plugin-sdk/icons/arrow-left';
  import FolderGit2Icon from '@mannie/plugin-sdk/icons/folder-git-2';
  import RefreshCwIcon from '@mannie/plugin-sdk/icons/refresh-cw';
  import type { SidePanelProps } from '@mannie/plugin-sdk/slots';
  import { untrack } from 'svelte';

  import type { Entry, RepositoryOverview } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { RepositoryPanel } from '../state/git-repository.svelte.js';

  import CommitList from './cards/CommitList.svelte';
  import ConflictFiles from './cards/ConflictFiles.svelte';
  import ConflictHunkView from './cards/ConflictHunkView.svelte';
  import DiffView from './cards/DiffView.svelte';
  import EntryList from './cards/EntryList.svelte';
  import RefList from './cards/RefList.svelte';
  import ReadFailure from './ReadFailure.svelte';

  // The Git side panel (plugins/git/docs/panel.md): a project's repository
  // beside the conversation. Its header names the branch, its upstream and
  // the operation in progress; its tabs show the changed files with their
  // diffs, the history with each commit's changes, the branches and tags,
  // and the conflicts of an operation that stopped. It only reads.
  let { argument, setLabel }: SidePanelProps = $props();

  let tab = $state('changes');

  const panel = $derived(new RepositoryPanel(argument));
  const overview = $derived(panel.overview.value);
  const tabLabel = $derived(overview === null ? null : folderName(overview.top));
  const failure = $derived(panel.overview.failure);
  const isNotARepository = $derived(
    failure?.kind === 'core' && failure.error.origin === 'plugin' && failure.error.code === 'not_a_repository',
  );
  const operation = $derived(overview === null ? null : operationLine(overview));
  const hasConflicts = $derived(
    overview !== null && (overview.operation !== null || overview.entries.some((entry) => entry.kind === 'conflicted')),
  );
  /** Whether the chosen tab is Conflicts, which went away with the conflicts. */
  const isTabGone = $derived(tab === 'conflicts' && !hasConflicts);
  const shownTab = $derived(isTabGone ? 'changes' : tab);

  function folderName(top: string): string {
    return top.split(/[\\/]/u).findLast((part) => part !== '') ?? top;
  }

  function branchLine(repository: RepositoryOverview): string {
    const branch = repository.branch.head;
    if (branch === null) {
      return m.git_panel_detached();
    }
    return repository.branch.oid === null ? m.git_panel_unborn() : m.git_panel_on_branch({ branch });
  }

  function trackingLine(repository: RepositoryOverview): string {
    const { ahead, behind, upstream } = repository.branch;
    if (upstream === null) {
      return m.git_panel_no_upstream();
    }
    const isCounted = ahead !== null && behind !== null;
    if (!isCounted) {
      return m.git_refs_gone({ upstream });
    }
    return m.git_refs_ahead_behind({ ahead: formatCount(ahead), behind: formatCount(behind), upstream });
  }

  function operationLine(repository: RepositoryOverview): string | null {
    const current = repository.operation;
    if (current === null) {
      return null;
    }
    switch (current.kind) {
      case 'cherry_pick': {
        return m.git_panel_operation_cherry_pick();
      }
      case 'merge': {
        return m.git_panel_operation_merge();
      }
      case 'rebase': {
        const step = current.step;
        return step === null
          ? m.git_panel_operation_rebase()
          : m.git_panel_operation_rebase_step({ step: formatCount(step.number), total: formatCount(step.total) });
      }
      case 'revert': {
        return m.git_panel_operation_revert();
      }
    }
  }

  /** Reads what a tab shows, the first time that it opens for the repository. */
  function readTab(repository: RepositoryPanel, name: string): void {
    switch (name) {
      case 'branches': {
        if (repository.branches.status === 'idle') {
          void repository.readBranches();
        }
        break;
      }
      case 'conflicts': {
        if (repository.conflicts.status === 'idle') {
          void repository.readConflicts();
        }
        break;
      }
      case 'history': {
        if (repository.historyPage.status === 'idle') {
          void repository.readHistory();
        }
        break;
      }
      default: {
        break;
      }
    }
  }

  function chooseFile(entry: Entry, staged: boolean): void {
    panel.chooseFile(entry.path, staged);
  }

  function chooseCommit(oid: string): void {
    panel.chooseCommit(oid);
  }

  function chooseTab(name: string): void {
    tab = name;
  }

  // Only another repository starts another watch. The watch's own reads of
  // the state stay untracked, or each value that it reads would restart it.
  $effect(() => {
    const watched = panel;
    return untrack(() => watched.watch());
  });

  // The shell keeps the tabs' labels; the panel tells it its own.
  $effect(() => {
    if (tabLabel !== null) {
      setLabel(tabLabel, true);
    }
  });

  // A tab reads what it shows when it first shows for a repository.
  $effect(() => {
    const name = shownTab;
    const shown = panel;
    untrack(() => {
      readTab(shown, name);
    });
  });
</script>

<div class="flex h-full min-h-0 flex-col gap-3 overflow-y-auto p-3" data-git-panel="">
  {#if isNotARepository}
    <Empty.Root>
      <Empty.Header>
        <Empty.Media variant="icon">
          <FolderGit2Icon />
        </Empty.Media>
        <Empty.Description>{m.git_error_not_a_repository()}</Empty.Description>
      </Empty.Header>
    </Empty.Root>
  {:else if overview === null}
    {#if failure === null}
      <div class="flex flex-col gap-3" aria-busy="true">
        <span class="sr-only" role="status">{m.git_panel_loading()}</span>
        <Skeleton class="h-6 w-1/2" />
        <Skeleton class="h-24 w-full" />
      </div>
    {:else}
      <ReadFailure {failure} retry={() => void panel.refresh()} title={m.git_panel_failed_title()} />
    {/if}
  {:else}
    <header class="flex items-start gap-2">
      <div class="flex min-w-0 flex-col">
        <p class="truncate font-medium" data-git-branch="">{branchLine(overview)}</p>
        <p class="truncate text-sm text-muted-foreground">{trackingLine(overview)}</p>
        {#if operation !== null}
          <p class="text-sm" data-git-operation="">{operation}</p>
        {/if}
        {#if overview.stash > 0}
          <p class="text-sm text-muted-foreground">{m.git_panel_stash({ count: overview.stash })}</p>
        {/if}
      </div>
      <Button
        class="ms-auto shrink-0"
        aria-label={m.git_panel_refresh()}
        size="icon-sm"
        title={m.git_panel_refresh()}
        variant="ghost"
        onclick={() => void panel.refresh()}
      >
        <RefreshCwIcon />
      </Button>
    </header>
    {#if failure !== null}
      <ReadFailure {failure} retry={() => void panel.refresh()} />
    {/if}
    <Tabs.Root bind:value={() => shownTab, chooseTab}>
      <Tabs.List aria-label={m.git_panel_tabs_label()}>
        <Tabs.Trigger value="changes">{m.git_panel_changes()}</Tabs.Trigger>
        <Tabs.Trigger value="history">{m.git_panel_history()}</Tabs.Trigger>
        <Tabs.Trigger value="branches">{m.git_panel_branches()}</Tabs.Trigger>
        {#if hasConflicts}
          <Tabs.Trigger value="conflicts">{m.git_panel_conflicts()}</Tabs.Trigger>
        {/if}
      </Tabs.List>
      <Tabs.Content class="flex flex-col gap-3" value="changes">
        {#if panel.file === null}
          <EntryList choose={chooseFile} entries={overview.entries} total={overview.entriesTotal} />
          {#if overview.entries.length > 0}
            <p class="text-sm text-muted-foreground">{m.git_panel_choose_file()}</p>
          {/if}
        {:else}
          <Button class="self-start" size="sm" variant="ghost" onclick={() => panel.closeFile()}>
            <ArrowLeftIcon data-icon="inline-start" />
            {m.git_panel_back()}
          </Button>
          {#if panel.fileDiff.failure !== null}
            <ReadFailure failure={panel.fileDiff.failure} retry={() => panel.readFile()} />
          {/if}
          {#if panel.fileDiff.value !== null}
            {const diff = $derived(panel.fileDiff.value)}
            {#if diff.stat === null}
              <p class="text-sm text-muted-foreground">{m.git_diff_no_changes()}</p>
            {:else}
              <DiffView files={[diff.stat]} open patches={[{ hunks: diff.hunks, path: diff.stat.path }]} />
            {/if}
          {:else if panel.fileDiff.status === 'loading'}
            <Skeleton class="h-24 w-full" />
          {/if}
        {/if}
      </Tabs.Content>
      <Tabs.Content class="flex flex-col gap-3" value="history">
        {#if panel.historyPage.failure !== null}
          <ReadFailure failure={panel.historyPage.failure} retry={() => void panel.readHistory()} />
        {/if}
        {#if panel.history.length > 0}
          <CommitList choose={chooseCommit} commits={panel.history} current={panel.chosenCommit} />
          {#if panel.historyMore}
            <Button class="self-start" size="sm" variant="outline" onclick={() => void panel.readHistory()}>
              {m.git_panel_show_more()}
            </Button>
          {/if}
          {#if panel.commit.failure !== null}
            <ReadFailure failure={panel.commit.failure} retry={() => panel.readCommit()} />
          {:else if panel.commit.value !== null}
            {const shown = $derived(panel.commit.value)}
            <section class="flex flex-col gap-2 border-t pt-3" aria-label={shown.commit.short}>
              <pre class="font-sans text-sm whitespace-pre-wrap">{shown.commit.message}</pre>
              <DiffView files={shown.files} patches={shown.patches} />
            </section>
          {:else if panel.commit.status === 'loading'}
            <Skeleton class="h-24 w-full" />
          {:else}
            <p class="text-sm text-muted-foreground">{m.git_panel_choose_commit()}</p>
          {/if}
        {:else if panel.historyPage.status === 'ready'}
          <p class="text-sm text-muted-foreground">{m.git_panel_history_empty()}</p>
        {:else if panel.historyPage.status === 'loading'}
          <Skeleton class="h-24 w-full" />
        {/if}
      </Tabs.Content>
      <Tabs.Content class="flex flex-col gap-3" value="branches">
        {#if panel.branches.failure !== null}
          <ReadFailure failure={panel.branches.failure} retry={() => void panel.readBranches()} />
        {/if}
        {#if panel.branches.value !== null}
          <RefList heading={m.git_refs_branches()} refs={panel.branches.value.branches} />
          <RefList heading={m.git_refs_remote_branches()} refs={panel.branches.value.remoteBranches} />
          <RefList heading={m.git_refs_tags()} refs={panel.branches.value.tags} />
        {:else if panel.branches.status === 'loading'}
          <Skeleton class="h-24 w-full" />
        {/if}
      </Tabs.Content>
      {#if hasConflicts}
        <Tabs.Content class="flex flex-col gap-3" value="conflicts">
          {#if panel.conflicts.failure !== null}
            <ReadFailure failure={panel.conflicts.failure} retry={() => void panel.readConflicts()} />
          {/if}
          {#if panel.conflicts.value === null}
            {#if panel.conflicts.status === 'loading'}
              <Skeleton class="h-24 w-full" />
            {/if}
          {:else if panel.conflicts.value.files.length === 0}
            <p class="text-sm text-muted-foreground">{m.git_panel_no_conflicts()}</p>
          {:else}
            {#each panel.conflicts.value.files as conflicted (conflicted.file.path)}
              <section class="flex flex-col gap-2">
                <ConflictFiles files={[conflicted.file]} />
                {#each conflicted.hunks as hunk (hunk.number)}
                  <ConflictHunkView {hunk} />
                {/each}
              </section>
            {/each}
          {/if}
        </Tabs.Content>
      {/if}
    </Tabs.Root>
  {/if}
</div>
