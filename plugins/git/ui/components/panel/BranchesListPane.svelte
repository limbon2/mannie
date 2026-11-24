<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Collapsible from '@mannie/plugin-sdk/components/collapsible';
  import * as DropdownMenu from '@mannie/plugin-sdk/components/dropdown-menu';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import ArchiveIcon from '@mannie/plugin-sdk/icons/archive';
  import ChevronRightIcon from '@mannie/plugin-sdk/icons/chevron-right';
  import CloudIcon from '@mannie/plugin-sdk/icons/cloud';
  import DownloadIcon from '@mannie/plugin-sdk/icons/download';
  import EllipsisVerticalIcon from '@mannie/plugin-sdk/icons/ellipsis-vertical';
  import FolderGit2Icon from '@mannie/plugin-sdk/icons/folder-git-2';
  import GitBranchIcon from '@mannie/plugin-sdk/icons/git-branch';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';
  import SearchIcon from '@mannie/plugin-sdk/icons/search';
  import TagIcon from '@mannie/plugin-sdk/icons/tag';
  import TrashIcon from '@mannie/plugin-sdk/icons/trash';
  import { SvelteMap } from 'svelte/reactivity';

  import type { Ref, ReflogEntry, Remote, Tracking, Worktree } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';
  import type { RepositoryPanel } from '../../state/git-repository.svelte.js';
  import { formatFullDate, formatRelativeTime, mentionEntryForBranch } from '../../utils/git-format.js';
  import { gitFailureMessage } from '../../utils/git-messages.js';

  // Searchable list pane for Local Branches, Stashes, Remote Branches grouped
  // by remote, newest-first Tags, Worktrees, and Remotes.
  interface Props {
    readonly panel: RepositoryPanel;
  }

  interface RemoteGroup {
    readonly branches: readonly Ref[];
    readonly remote: string;
  }

  let { panel }: Props = $props();

  let filterText = $state('');

  const refs = $derived(panel.refs.value);
  const needle = $derived(filterText.trim().toLowerCase());
  const isLoadingInitial = $derived(panel.refs.status === 'loading' && refs === null);
  const loadFailure = $derived(panel.refs.status === 'error' ? panel.refs.failure : null);
  const localBranches = $derived((refs?.branches ?? []).filter(matchesRef));
  const remoteGroups: readonly RemoteGroup[] = $derived.by(() => {
    const groups = new SvelteMap<string, Ref[]>();
    for (const refItem of refs?.remoteBranches ?? []) {
      if (!matchesRef(refItem)) {
        continue;
      }
      const slash = refItem.name.indexOf('/');
      const remote = slash === -1 ? 'origin' : refItem.name.slice(0, slash);
      const list = groups.get(remote);
      if (list === undefined) {
        groups.set(remote, [refItem]);
      } else {
        list.push(refItem);
      }
    }
    return [...groups.entries()].map(([remote, branches]) => ({ branches, remote }));
  });
  const tags = $derived((refs?.tags ?? []).filter(matchesRef));
  const stashes = $derived((refs?.stashes ?? []).filter(matchesStash));
  const worktrees = $derived((refs?.worktrees ?? []).filter(matchesWorktree));
  const remotes = $derived((refs?.remotes ?? []).filter(matchesRemote));

  function matchesText(value: string): boolean {
    const isEmpty = needle.length === 0;
    const includesNeedle = value.toLowerCase().includes(needle);
    return isEmpty || includesNeedle;
  }

  function matchesRef(refItem: Ref): boolean {
    return matchesText(refItem.name) || matchesText(refItem.subject);
  }

  function matchesStash(entry: ReflogEntry): boolean {
    return matchesText(entry.selector) || matchesText(entry.action);
  }

  function matchesWorktree(wt: Worktree): boolean {
    if (matchesText(wt.path)) {
      return true;
    }
    if (wt.branch === null) {
      return false;
    }
    return matchesText(wt.branch);
  }

  function matchesRemote(remote: Remote): boolean {
    if (matchesText(remote.name)) {
      return true;
    }
    if (remote.fetch !== null) {
      if (matchesText(remote.fetch)) {
        return true;
      }
    }
    if (remote.push === null) {
      return false;
    }
    return matchesText(remote.push);
  }

  function hasAheadOrBehind(tracking: Tracking): boolean {
    if (tracking.state === 'gone') {
      return false;
    }
    const hasAhead = tracking.ahead > 0;
    const hasBehind = tracking.behind > 0;
    return hasAhead || hasBehind;
  }

  function stashIndexOf(entry: ReflogEntry, fallback: number): number {
    const match = /^stash@\{(\d+)\}$/.exec(entry.selector);
    const parsed = match?.[1];
    return parsed === undefined ? fallback : Number.parseInt(parsed, 10);
  }

  function switchBranch(name: string): void {
    void panel.runAction({ create: false, detach: false, kind: 'switch_branch', start: null, target: name });
  }

  function checkoutRemoteBranch(remoteName: string, refItem: Ref): void {
    const prefix = `${remoteName}/`;
    const shortName = refItem.name.startsWith(prefix) ? refItem.name.slice(prefix.length) : refItem.name;
    const hasLocalCollision = refs?.branches.some((branch) => branch.name === shortName) ?? false;
    if (hasLocalCollision) {
      panel.openDialog({ kind: 'create_branch', start: refItem.name });
      return;
    }
    void panel.runAction({
      kind: 'checkout_remote_branch',
      localName: shortName,
      remoteRef: refItem.name,
    });
  }

  function mergeBranch(branch: string): void {
    void panel.runAction({ branch, kind: 'merge', message: null, mode: 'default' });
  }

  function rebaseOnto(onto: string): void {
    void panel.runAction({ autosquash: false, kind: 'rebase', onto });
  }

  function deleteBranch(name: string): void {
    void panel.runAction({ kind: 'delete_branch', name });
  }

  function deleteTag(name: string): void {
    void panel.runAction({ kind: 'delete_tag', name });
  }

  function applyStash(index: number): void {
    void panel.runAction({ index, kind: 'stash_apply' });
  }

  function popStash(index: number): void {
    void panel.runAction({ index, kind: 'stash_pop' });
  }

  function dropStash(index: number): void {
    void panel.runAction({ index, kind: 'stash_drop' });
  }

  function fetchRemote(remote: string): void {
    void panel.runAction({ kind: 'fetch', prune: false, remote });
  }

  function toggleStash(index: number): void {
    if (panel.chosenStashIndex === index) {
      panel.closeStash();
    } else {
      panel.chooseStash(index);
    }
  }
</script>

<div class="flex h-full min-h-0 flex-col overflow-hidden">
  <div class="flex flex-wrap items-center gap-1.5 border-b px-2.5 py-2">
    <div class="relative min-w-32 flex-1">
      <SearchIcon
        class="pointer-events-none absolute inset-s-2 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground"
        aria-hidden="true"
      />
      <Input
        class="h-7 ps-7 text-xs"
        aria-label={m.git_refs_filter_placeholder()}
        placeholder={m.git_refs_filter_placeholder()}
        bind:value={filterText}
      />
    </div>

    <Button
      class="h-7 px-2 text-xs"
      disabled={panel.runningAction}
      size="sm"
      variant="outline"
      onclick={() => panel.openDialog({ kind: 'create_branch', start: 'HEAD' })}
    >
      <PlusIcon class="size-3.5" aria-hidden="true" />
      <span>{m.git_refs_new_branch()}</span>
    </Button>

    <Button
      class="h-7 px-2 text-xs"
      disabled={panel.runningAction}
      size="sm"
      variant="ghost"
      onclick={() => panel.openDialog({ kind: 'create_tag', target: 'HEAD' })}
    >
      <TagIcon class="size-3.5" aria-hidden="true" />
      <span>{m.git_refs_new_tag()}</span>
    </Button>

    <Button
      class="h-7 px-2 text-xs"
      disabled={panel.runningAction}
      size="sm"
      variant="ghost"
      onclick={() => panel.openDialog({ kind: 'stash_push' })}
    >
      <ArchiveIcon class="size-3.5" aria-hidden="true" />
      <span>{m.git_refs_new_stash()}</span>
    </Button>
  </div>

  <div class="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-2.5">
    {#if isLoadingInitial}
      <div class="flex items-center gap-2 py-4 text-xs text-muted-foreground" aria-busy="true">
        <Spinner aria-label={m.git_panel_loading()} />
        <span>{m.git_panel_loading()}</span>
      </div>
    {:else if loadFailure !== null}
      <Alert.Root variant="destructive">
        <Alert.Description class="flex items-center justify-between gap-2 text-xs">
          <span>{gitFailureMessage(loadFailure)}</span>
          <Button size="sm" variant="outline" onclick={() => panel.readRefs()}>{m.git_panel_try_again()}</Button>
        </Alert.Description>
      </Alert.Root>
    {:else if refs !== null}
      <!-- Local Branches -->
      <Collapsible.Root class="group/section flex flex-col gap-1" open={true}>
        <Collapsible.Trigger
          class="flex items-center gap-1.5 text-xs font-medium text-muted-foreground hover:text-foreground"
        >
          <ChevronRightIcon
            class="size-3.5 shrink-0 transition-transform group-data-[state=open]/section:rotate-90"
            aria-hidden="true"
          />
          <GitBranchIcon class="size-3.5 shrink-0" aria-hidden="true" />
          <span>{m.git_refs_branches()}</span>
          <Badge class="h-4 px-1.5 text-[10px]" variant="secondary">{formatCount(localBranches.length)}</Badge>
        </Collapsible.Trigger>

        <Collapsible.Content>
          {#if localBranches.length === 0}
            <p class="p-2 text-xs text-muted-foreground">{m.git_refs_empty()}</p>
          {:else}
            <ul class="flex flex-col divide-y rounded-md border">
              {#each localBranches as branch (branch.name)}
                <li class="flex min-w-0 items-start gap-2 px-2.5 py-1.5 text-xs hover:bg-muted/40">
                  <div class="flex min-w-0 flex-1 flex-col gap-0.5">
                    <div class="flex min-w-0 items-center gap-1.5">
                      <span class="min-w-0 truncate font-mono font-medium" title={branch.name}>{branch.name}</span>
                      {#if branch.current}
                        <Badge class="h-4 shrink-0 px-1.5 text-[10px]" variant="default">
                          {m.git_refs_current()}
                        </Badge>
                      {/if}
                      {#if branch.tracking !== null}
                        {#if branch.tracking.state === 'gone'}
                          <Badge class="h-4 shrink-0 px-1.5 text-[10px]" variant="outline">
                            {m.git_refs_gone({ upstream: branch.upstream ?? '' })}
                          </Badge>
                        {:else if hasAheadOrBehind(branch.tracking)}
                          <span
                            class="flex shrink-0 items-center gap-1 text-[11px] text-muted-foreground tabular-nums"
                            title={m.git_refs_ahead_behind({
                              ahead: formatCount(branch.tracking.ahead),
                              behind: formatCount(branch.tracking.behind),
                              upstream: branch.upstream ?? '',
                            })}
                          >
                            {#if branch.tracking.ahead > 0}
                              <span>{m.git_header_ahead({ count: formatCount(branch.tracking.ahead) })}</span>
                            {/if}
                            {#if branch.tracking.behind > 0}
                              <span>{m.git_header_behind({ count: formatCount(branch.tracking.behind) })}</span>
                            {/if}
                          </span>
                        {/if}
                      {/if}
                      <span class="ms-auto shrink-0 font-mono text-[11px] text-muted-foreground">{branch.short}</span>
                    </div>

                    <div class="flex min-w-0 items-center gap-2 text-muted-foreground">
                      <span class="min-w-0 flex-1 truncate">{branch.subject}</span>
                      {#if branch.date.length > 0}
                        <span class="shrink-0 text-[11px] tabular-nums" title={formatFullDate(branch.date)}>
                          {formatRelativeTime(branch.date)}
                        </span>
                      {/if}
                    </div>
                  </div>

                  <DropdownMenu.Root>
                    <DropdownMenu.Trigger>
                      {#snippet child({ props })}
                        <Button
                          {...props}
                          class="size-6 shrink-0"
                          aria-label={m.git_refs_actions({ name: branch.name })}
                          disabled={panel.runningAction}
                          size="icon"
                          variant="ghost"
                        >
                          <EllipsisVerticalIcon class="size-3.5" aria-hidden="true" />
                        </Button>
                      {/snippet}
                    </DropdownMenu.Trigger>
                    <DropdownMenu.Content align="end">
                      {#if !branch.current}
                        <DropdownMenu.Item onclick={() => switchBranch(branch.name)}>
                          {m.git_refs_switch()}
                        </DropdownMenu.Item>
                        <DropdownMenu.Item onclick={() => mergeBranch(branch.name)}>
                          {m.git_refs_merge_into_current()}
                        </DropdownMenu.Item>
                        <DropdownMenu.Item onclick={() => rebaseOnto(branch.name)}>
                          {m.git_refs_rebase_onto()}
                        </DropdownMenu.Item>
                        <DropdownMenu.Separator />
                      {/if}
                      <DropdownMenu.Item onclick={() => panel.openCompare(branch.name, 'HEAD')}>
                        {m.git_refs_compare_with_head()}
                      </DropdownMenu.Item>
                      <DropdownMenu.Item onclick={() => panel.mentionInChat(mentionEntryForBranch(branch))}>
                        {m.git_panel_mention_branch()}
                      </DropdownMenu.Item>
                      <DropdownMenu.Separator />
                      <DropdownMenu.Item onclick={() => panel.openDialog({ kind: 'rename_branch', name: branch.name })}>
                        {m.git_refs_rename()}
                      </DropdownMenu.Item>
                      {#if !branch.current}
                        <DropdownMenu.Item variant="destructive" onclick={() => deleteBranch(branch.name)}>
                          {m.git_refs_delete()}
                        </DropdownMenu.Item>
                      {/if}
                    </DropdownMenu.Content>
                  </DropdownMenu.Root>
                </li>
              {/each}
            </ul>
          {/if}
        </Collapsible.Content>
      </Collapsible.Root>

      <!-- Stashes -->
      <Collapsible.Root class="group/section flex flex-col gap-1" open={stashes.length > 0}>
        <Collapsible.Trigger
          class="flex items-center gap-1.5 text-xs font-medium text-muted-foreground hover:text-foreground"
        >
          <ChevronRightIcon
            class="size-3.5 shrink-0 transition-transform group-data-[state=open]/section:rotate-90"
            aria-hidden="true"
          />
          <ArchiveIcon class="size-3.5 shrink-0" aria-hidden="true" />
          <span>{m.git_refs_stashes()}</span>
          <Badge class="h-4 px-1.5 text-[10px]" variant="secondary">{formatCount(stashes.length)}</Badge>
        </Collapsible.Trigger>

        <Collapsible.Content>
          {#if stashes.length === 0}
            <p class="p-2 text-xs text-muted-foreground">{m.git_refs_stash_empty()}</p>
          {:else}
            <ul class="flex flex-col divide-y rounded-md border">
              {#each stashes as entry, idx (entry.selector)}
                {const stashIdx = $derived(stashIndexOf(entry, idx))}
                {const selected = $derived(panel.chosenStashIndex === stashIdx)}
                <li
                  class={['flex min-w-0 items-center gap-1.5 pe-1.5', selected ? 'bg-accent/50' : 'hover:bg-muted/40']}
                >
                  <button
                    class="flex min-w-0 flex-1 cursor-pointer flex-col gap-0.5 px-2.5 py-1.5 text-start text-xs"
                    aria-current={selected ? 'true' : undefined}
                    type="button"
                    onclick={() => toggleStash(stashIdx)}
                  >
                    <div class="flex items-center gap-1.5">
                      <Badge class="h-4 shrink-0 font-mono text-[10px]" variant="outline">{entry.selector}</Badge>
                      <span class="min-w-0 flex-1 truncate font-medium">{entry.action}</span>
                    </div>
                    <span
                      class="text-[11px] text-muted-foreground tabular-nums"
                      title={formatFullDate(entry.committed)}
                    >
                      {formatRelativeTime(entry.committed)}
                    </span>
                  </button>

                  <div class="flex shrink-0 items-center gap-0.5">
                    <Button
                      class="h-6 px-1.5 text-[11px]"
                      disabled={panel.runningAction}
                      size="sm"
                      variant="ghost"
                      onclick={() => applyStash(stashIdx)}
                    >
                      {m.git_refs_stash_apply()}
                    </Button>
                    <Button
                      class="h-6 px-1.5 text-[11px]"
                      disabled={panel.runningAction}
                      size="sm"
                      variant="ghost"
                      onclick={() => popStash(stashIdx)}
                    >
                      {m.git_refs_stash_pop()}
                    </Button>
                    <Button
                      class="size-6 text-destructive hover:text-destructive"
                      aria-label={m.git_refs_stash_drop()}
                      disabled={panel.runningAction}
                      size="icon"
                      title={m.git_refs_stash_drop()}
                      variant="ghost"
                      onclick={() => dropStash(stashIdx)}
                    >
                      <TrashIcon class="size-3" aria-hidden="true" />
                    </Button>
                  </div>
                </li>
              {/each}
            </ul>
          {/if}
        </Collapsible.Content>
      </Collapsible.Root>

      <!-- Remote Branches -->
      {#if remoteGroups.length > 0}
        <Collapsible.Root class="group/section flex flex-col gap-1" open={false}>
          <Collapsible.Trigger
            class="flex items-center gap-1.5 text-xs font-medium text-muted-foreground hover:text-foreground"
          >
            <ChevronRightIcon
              class="size-3.5 shrink-0 transition-transform group-data-[state=open]/section:rotate-90"
              aria-hidden="true"
            />
            <CloudIcon class="size-3.5 shrink-0" aria-hidden="true" />
            <span>{m.git_refs_remote_branches()}</span>
            <Badge class="h-4 px-1.5 text-[10px]" variant="secondary">
              {formatCount(refs.remoteBranches.length)}
            </Badge>
          </Collapsible.Trigger>

          <Collapsible.Content class="flex flex-col gap-2">
            {#each remoteGroups as group (group.remote)}
              <div class="flex flex-col gap-1">
                <div class="px-1 text-[11px] font-medium text-muted-foreground">{group.remote}</div>
                <ul class="flex flex-col divide-y rounded-md border">
                  {#each group.branches as remoteBranch (remoteBranch.name)}
                    <li class="flex min-w-0 items-center gap-2 px-2.5 py-1.5 text-xs hover:bg-muted/40">
                      <div class="flex min-w-0 flex-1 flex-col gap-0.5">
                        <div class="flex items-center gap-1.5">
                          <span class="min-w-0 flex-1 truncate font-mono" title={remoteBranch.name}>
                            {remoteBranch.name}
                          </span>
                          <span class="shrink-0 font-mono text-[11px] text-muted-foreground">
                            {remoteBranch.short}
                          </span>
                        </div>
                        <span class="truncate text-muted-foreground">{remoteBranch.subject}</span>
                      </div>

                      <DropdownMenu.Root>
                        <DropdownMenu.Trigger>
                          {#snippet child({ props })}
                            <Button
                              {...props}
                              class="size-6 shrink-0"
                              aria-label={m.git_refs_actions({ name: remoteBranch.name })}
                              disabled={panel.runningAction}
                              size="icon"
                              variant="ghost"
                            >
                              <EllipsisVerticalIcon class="size-3.5" aria-hidden="true" />
                            </Button>
                          {/snippet}
                        </DropdownMenu.Trigger>
                        <DropdownMenu.Content align="end">
                          <DropdownMenu.Item onclick={() => checkoutRemoteBranch(group.remote, remoteBranch)}>
                            {m.git_refs_switch()}
                          </DropdownMenu.Item>
                          <DropdownMenu.Item onclick={() => mergeBranch(remoteBranch.name)}>
                            {m.git_refs_merge_into_current()}
                          </DropdownMenu.Item>
                          <DropdownMenu.Item onclick={() => panel.openCompare(remoteBranch.name, 'HEAD')}>
                            {m.git_refs_compare_with_head()}
                          </DropdownMenu.Item>
                        </DropdownMenu.Content>
                      </DropdownMenu.Root>
                    </li>
                  {/each}
                </ul>
              </div>
            {/each}
          </Collapsible.Content>
        </Collapsible.Root>
      {/if}

      <!-- Tags (newest first) -->
      {#if tags.length > 0}
        <Collapsible.Root class="group/section flex flex-col gap-1" open={false}>
          <Collapsible.Trigger
            class="flex items-center gap-1.5 text-xs font-medium text-muted-foreground hover:text-foreground"
          >
            <ChevronRightIcon
              class="size-3.5 shrink-0 transition-transform group-data-[state=open]/section:rotate-90"
              aria-hidden="true"
            />
            <TagIcon class="size-3.5 shrink-0" aria-hidden="true" />
            <span>{m.git_refs_tags()}</span>
            <Badge class="h-4 px-1.5 text-[10px]" variant="secondary">{formatCount(tags.length)}</Badge>
          </Collapsible.Trigger>

          <Collapsible.Content>
            <ul class="flex flex-col divide-y rounded-md border">
              {#each tags as tagItem (tagItem.name)}
                <li class="flex min-w-0 items-center gap-2 px-2.5 py-1.5 text-xs hover:bg-muted/40">
                  <div class="flex min-w-0 flex-1 flex-col gap-0.5">
                    <div class="flex items-center gap-1.5">
                      <span class="min-w-0 flex-1 truncate font-mono font-medium" title={tagItem.name}>
                        {tagItem.name}
                      </span>
                      <span class="shrink-0 font-mono text-[11px] text-muted-foreground">{tagItem.short}</span>
                    </div>
                    <div class="flex items-center gap-2 text-muted-foreground">
                      <span class="min-w-0 flex-1 truncate">{tagItem.subject}</span>
                      {#if tagItem.date.length > 0}
                        <span class="shrink-0 text-[11px] tabular-nums" title={formatFullDate(tagItem.date)}>
                          {formatRelativeTime(tagItem.date)}
                        </span>
                      {/if}
                    </div>
                  </div>

                  <DropdownMenu.Root>
                    <DropdownMenu.Trigger>
                      {#snippet child({ props })}
                        <Button
                          {...props}
                          class="size-6 shrink-0"
                          aria-label={m.git_refs_actions({ name: tagItem.name })}
                          disabled={panel.runningAction}
                          size="icon"
                          variant="ghost"
                        >
                          <EllipsisVerticalIcon class="size-3.5" aria-hidden="true" />
                        </Button>
                      {/snippet}
                    </DropdownMenu.Trigger>
                    <DropdownMenu.Content align="end">
                      <DropdownMenu.Item onclick={() => panel.openCommitInGraph(tagItem.commit)}>
                        {m.git_inspect_inspect_commit()}
                      </DropdownMenu.Item>
                      <DropdownMenu.Item onclick={() => panel.openCompare(tagItem.name, 'HEAD')}>
                        {m.git_refs_compare_with_head()}
                      </DropdownMenu.Item>
                      <DropdownMenu.Item
                        onclick={() => panel.openDialog({ kind: 'create_branch', start: tagItem.name })}
                      >
                        {m.git_commit_create_branch()}
                      </DropdownMenu.Item>
                      <DropdownMenu.Separator />
                      <DropdownMenu.Item variant="destructive" onclick={() => deleteTag(tagItem.name)}>
                        {m.git_refs_delete()}
                      </DropdownMenu.Item>
                    </DropdownMenu.Content>
                  </DropdownMenu.Root>
                </li>
              {/each}
            </ul>
          </Collapsible.Content>
        </Collapsible.Root>
      {/if}

      <!-- Worktrees -->
      {#if worktrees.length > 1}
        <Collapsible.Root class="group/section flex flex-col gap-1" open={false}>
          <Collapsible.Trigger
            class="flex items-center gap-1.5 text-xs font-medium text-muted-foreground hover:text-foreground"
          >
            <ChevronRightIcon
              class="size-3.5 shrink-0 transition-transform group-data-[state=open]/section:rotate-90"
              aria-hidden="true"
            />
            <FolderGit2Icon class="size-3.5 shrink-0" aria-hidden="true" />
            <span>{m.git_refs_worktrees()}</span>
            <Badge class="h-4 px-1.5 text-[10px]" variant="secondary">{formatCount(worktrees.length)}</Badge>
          </Collapsible.Trigger>

          <Collapsible.Content>
            <ul class="flex flex-col divide-y rounded-md border">
              {#each worktrees as wt (wt.path)}
                <li class="flex flex-col gap-0.5 px-2.5 py-1.5 text-xs">
                  <div class="flex items-center gap-1.5">
                    <span class="min-w-0 flex-1 truncate font-mono" title={wt.path}>{wt.path}</span>
                    {#if wt.branch !== null}
                      <Badge class="h-4 px-1.5 font-mono text-[10px]" variant="secondary">{wt.branch}</Badge>
                    {/if}
                    {#if wt.bare}
                      <Badge class="h-4 px-1.5 text-[10px]" variant="outline">{m.git_refs_worktree_bare()}</Badge>
                    {/if}
                    {#if wt.locked}
                      <Badge class="h-4 px-1.5 text-[10px]" variant="outline">{m.git_refs_worktree_locked()}</Badge>
                    {/if}
                    {#if wt.prunable}
                      <Badge class="h-4 px-1.5 text-[10px]" variant="destructive">
                        {m.git_refs_worktree_prunable()}
                      </Badge>
                    {/if}
                  </div>
                </li>
              {/each}
            </ul>
          </Collapsible.Content>
        </Collapsible.Root>
      {/if}

      <!-- Remotes -->
      {#if remotes.length > 0}
        <Collapsible.Root class="group/section flex flex-col gap-1" open={false}>
          <Collapsible.Trigger
            class="flex items-center gap-1.5 text-xs font-medium text-muted-foreground hover:text-foreground"
          >
            <ChevronRightIcon
              class="size-3.5 shrink-0 transition-transform group-data-[state=open]/section:rotate-90"
              aria-hidden="true"
            />
            <CloudIcon class="size-3.5 shrink-0" aria-hidden="true" />
            <span>{m.git_refs_remotes()}</span>
            <Badge class="h-4 px-1.5 text-[10px]" variant="secondary">{formatCount(remotes.length)}</Badge>
          </Collapsible.Trigger>

          <Collapsible.Content>
            <ul class="flex flex-col divide-y rounded-md border">
              {#each remotes as remote (remote.name)}
                <li class="flex items-center gap-2 px-2.5 py-1.5 text-xs">
                  <div class="flex min-w-0 flex-1 flex-col gap-0.5">
                    <span class="font-mono font-medium">{remote.name}</span>
                    {#if remote.fetch !== null}
                      <span class="truncate font-mono text-[11px] text-muted-foreground" title={remote.fetch}>
                        {remote.fetch}
                      </span>
                    {/if}
                  </div>
                  <Button
                    class="h-6 px-2 text-xs"
                    disabled={panel.runningAction}
                    size="sm"
                    variant="outline"
                    onclick={() => fetchRemote(remote.name)}
                  >
                    <DownloadIcon class="size-3" aria-hidden="true" />
                    <span>{m.git_refs_remote_fetch()}</span>
                  </Button>
                </li>
              {/each}
            </ul>
          </Collapsible.Content>
        </Collapsible.Root>
      {/if}
    {/if}
  </div>
</div>
