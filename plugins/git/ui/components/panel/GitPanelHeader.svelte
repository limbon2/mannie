<script lang="ts">
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Command from '@mannie/plugin-sdk/components/command';
  import * as DropdownMenu from '@mannie/plugin-sdk/components/dropdown-menu';
  import * as Popover from '@mannie/plugin-sdk/components/popover';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import ArrowDownToLineIcon from '@mannie/plugin-sdk/icons/arrow-down-to-line';
  import ArrowUpFromLineIcon from '@mannie/plugin-sdk/icons/arrow-up-from-line';
  import AtSignIcon from '@mannie/plugin-sdk/icons/at-sign';
  import CheckIcon from '@mannie/plugin-sdk/icons/check';
  import ChevronDownIcon from '@mannie/plugin-sdk/icons/chevron-down';
  import DownloadIcon from '@mannie/plugin-sdk/icons/download';
  import EllipsisVerticalIcon from '@mannie/plugin-sdk/icons/ellipsis-vertical';
  import GitBranchIcon from '@mannie/plugin-sdk/icons/git-branch';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';
  import RefreshCwIcon from '@mannie/plugin-sdk/icons/refresh-cw';
  import TagIcon from '@mannie/plugin-sdk/icons/tag';

  import type { RepositoryOverview } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';
  import type { RepositoryPanel } from '../../state/git-repository.svelte.js';
  import { mentionEntryForChanges } from '../../utils/git-format.js';

  // Compact panel header: searchable branch quick-switcher (preserving the
  // branch name on unborn repos), upstream ahead/behind sync pill, Fetch /
  // Pull / Push actions with remote URL tooltips, Mention in chat, and Refresh.
  interface Props {
    readonly overview: RepositoryOverview;
    readonly panel: RepositoryPanel;
  }

  interface PushRemoteOptions {
    readonly forceWithLease?: boolean;
    readonly setUpstream?: boolean;
    readonly tags?: boolean;
  }

  let { overview, panel }: Props = $props();

  let switcherOpen = $state(false);
  const switcherListId = 'git-branch-switcher-list';

  const branch = $derived(overview.branch);
  const isUnborn = $derived(branch.head !== null && branch.oid === null);
  const detachedShort = $derived(branch.head === null ? (overview.head?.short ?? null) : null);
  const canPushBranch = $derived(branch.head !== null && !isUnborn);
  const branchHeading = $derived(
    branch.head !== null ? m.git_panel_on_branch({ branch: branch.head }) : m.git_panel_detached(),
  );
  const pushTarget = $derived(overview.pushTarget);
  const remoteTooltip = $derived.by(() => {
    if (pushTarget === null) {
      return undefined;
    }
    const pushUrl = pushTarget.push ?? '';
    if (pushUrl.length === 0) {
      return undefined;
    }
    return m.git_header_remote_url({
      remote: pushTarget.name,
      url: pushUrl,
    });
  });
  const hasUpstream = $derived(branch.upstream !== null);
  const isUpToDate = $derived(hasUpstream && branch.ahead === 0 && branch.behind === 0);
  const isUpstreamGone = $derived(hasUpstream && (branch.ahead === null || branch.behind === null));
  const pushLabel = $derived(hasUpstream ? m.git_header_push() : m.git_header_publish());
  const pushTitle = $derived(remoteTooltip === undefined ? pushLabel : `${pushLabel} · ${remoteTooltip}`);
  const hasChanges = $derived(overview.entriesTotal > 0);
  const refs = $derived(panel.refs.value);
  const localBranches = $derived(refs?.branches ?? []);
  const tagList = $derived(refs?.tags.slice(0, 20) ?? []);

  function handleSwitcherOpenChange(open: boolean): void {
    switcherOpen = open;
    const isRefsIdle = panel.refs.status === 'idle';
    if (open && isRefsIdle) {
      void panel.readRefs();
    }
  }

  function switchToBranch(target: string): void {
    switcherOpen = false;
    void panel.runAction({ create: false, detach: false, kind: 'switch_branch', start: null, target });
  }

  function switchToTag(target: string): void {
    switcherOpen = false;
    void panel.runAction({ create: false, detach: true, kind: 'switch_branch', start: null, target });
  }

  function openCreateBranch(): void {
    switcherOpen = false;
    panel.openDialog({ kind: 'create_branch', start: branch.head ?? 'HEAD' });
  }

  function fetchRemote(prune = false): void {
    void panel.runAction({ kind: 'fetch', prune, remote: null });
  }

  function pullRemote(integrate: 'merge' | 'rebase'): void {
    void panel.runAction({ integrate, kind: 'pull', prune: false, remote: null });
  }

  function pushRemote(options: PushRemoteOptions = {}): void {
    void panel.runAction({
      branch: null,
      forceWithLease: options.forceWithLease ?? false,
      kind: 'push',
      remote: null,
      setUpstream: options.setUpstream ?? false,
      tags: options.tags ?? false,
    });
  }

  function mentionChanges(): void {
    panel.mentionInChat(mentionEntryForChanges());
  }

  function openStashes(): void {
    panel.activeTab = 'branches';
    if (panel.refs.status === 'idle') {
      void panel.readRefs();
    }
  }

  function refreshAll(): void {
    void panel.refresh();
  }
</script>

<header class="flex flex-col gap-1.5 border-b px-3 py-2">
  <div class="flex min-w-0 items-center gap-1.5">
    <Popover.Root open={switcherOpen} onOpenChange={handleSwitcherOpenChange}>
      <Popover.Trigger>
        {#snippet child({ props })}
          <Button
            {...props}
            class="h-7 max-w-full min-w-0 gap-1.5 px-2 font-medium"
            data-git-branch=""
            size="sm"
            title={m.git_header_switch_branch()}
            variant="ghost"
          >
            <GitBranchIcon class="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
            <span class="truncate">{branchHeading}</span>
            <ChevronDownIcon class="size-3 shrink-0 text-muted-foreground" aria-hidden="true" />
          </Button>
        {/snippet}
      </Popover.Trigger>

      <Popover.Content class="w-72 p-0" align="start">
        <Command.Root>
          <Command.Input aria-controls={switcherListId} placeholder={m.git_header_search_branches()} />
          <Command.List id={switcherListId} aria-label={m.git_header_switch_branch()}>
            <Command.Empty>{m.git_header_no_matching_refs()}</Command.Empty>

            <Command.Group>
              <Command.Item value="__create_branch" onSelect={openCreateBranch}>
                <PlusIcon class="size-3.5" aria-hidden="true" />
                <span>{m.git_header_create_branch_action()}</span>
              </Command.Item>
            </Command.Group>

            {#if localBranches.length > 0}
              <Command.Separator />
              <Command.Group heading={m.git_refs_branches()}>
                {#each localBranches as refItem (refItem.name)}
                  <Command.Item value={refItem.name} onSelect={() => switchToBranch(refItem.name)}>
                    <GitBranchIcon class="size-3.5 shrink-0" aria-hidden="true" />
                    <span class="min-w-0 flex-1 truncate font-mono text-xs">{refItem.name}</span>
                    {#if refItem.current}
                      <CheckIcon class="size-3.5 shrink-0" aria-hidden="true" />
                    {/if}
                  </Command.Item>
                {/each}
              </Command.Group>
            {/if}

            {#if tagList.length > 0}
              <Command.Separator />
              <Command.Group heading={m.git_refs_tags()}>
                {#each tagList as tagItem (tagItem.name)}
                  <Command.Item value={`tag:${tagItem.name}`} onSelect={() => switchToTag(tagItem.name)}>
                    <TagIcon class="size-3.5 shrink-0" aria-hidden="true" />
                    <span class="min-w-0 flex-1 truncate font-mono text-xs">{tagItem.name}</span>
                  </Command.Item>
                {/each}
              </Command.Group>
            {/if}
          </Command.List>
        </Command.Root>
      </Popover.Content>
    </Popover.Root>

    {#if isUnborn}
      <Badge class="shrink-0" variant="outline">{m.git_panel_unborn()}</Badge>
    {:else if detachedShort !== null}
      <Badge class="shrink-0 font-mono" variant="outline">{detachedShort}</Badge>
    {/if}

    <div class="ms-auto flex shrink-0 items-center gap-0.5">
      {#if overview.remotes.length > 0}
        <Button
          class="size-7"
          aria-label={m.git_header_fetch()}
          disabled={panel.runningAction}
          size="icon"
          title={remoteTooltip === undefined ? m.git_header_fetch() : `${m.git_header_fetch()} · ${remoteTooltip}`}
          variant="ghost"
          onclick={() => fetchRemote(false)}
        >
          <DownloadIcon class="size-3.5" aria-hidden="true" />
        </Button>

        {#if hasUpstream && !isUpstreamGone}
          <Button
            class="size-7"
            aria-label={m.git_header_pull()}
            disabled={panel.runningAction}
            size="icon"
            title={remoteTooltip === undefined ? m.git_header_pull() : `${m.git_header_pull()} · ${remoteTooltip}`}
            variant="ghost"
            onclick={() => pullRemote('merge')}
          >
            <ArrowDownToLineIcon class="size-3.5" aria-hidden="true" />
          </Button>
        {/if}

        {#if canPushBranch}
          <Button
            class="size-7"
            aria-label={pushLabel}
            disabled={panel.runningAction}
            size="icon"
            title={pushTitle}
            variant="ghost"
            onclick={() => pushRemote({ setUpstream: !hasUpstream })}
          >
            <ArrowUpFromLineIcon class="size-3.5" aria-hidden="true" />
          </Button>
        {/if}

        <DropdownMenu.Root>
          <DropdownMenu.Trigger>
            {#snippet child({ props })}
              <Button
                {...props}
                class="size-7"
                aria-label={m.git_header_more_sync()}
                disabled={panel.runningAction}
                size="icon"
                title={m.git_header_more_sync()}
                variant="ghost"
              >
                <EllipsisVerticalIcon class="size-3.5" aria-hidden="true" />
              </Button>
            {/snippet}
          </DropdownMenu.Trigger>
          <DropdownMenu.Content align="end">
            <DropdownMenu.Item onclick={() => fetchRemote(false)}>{m.git_header_fetch()}</DropdownMenu.Item>
            <DropdownMenu.Item onclick={() => fetchRemote(true)}>{m.git_header_fetch_prune()}</DropdownMenu.Item>
            {#if hasUpstream && !isUpstreamGone}
              <DropdownMenu.Separator />
              <DropdownMenu.Item onclick={() => pullRemote('merge')}>{m.git_header_pull_merge()}</DropdownMenu.Item>
              <DropdownMenu.Item onclick={() => pullRemote('rebase')}>{m.git_header_pull_rebase()}</DropdownMenu.Item>
            {/if}
            {#if canPushBranch}
              <DropdownMenu.Separator />
              <DropdownMenu.Item onclick={() => pushRemote({ setUpstream: !hasUpstream })}>
                {hasUpstream ? m.git_header_push() : m.git_header_push_set_upstream()}
              </DropdownMenu.Item>
              <DropdownMenu.Item onclick={() => pushRemote({ tags: true })}>
                {m.git_header_push_tags()}
              </DropdownMenu.Item>
              <DropdownMenu.Item variant="destructive" onclick={() => pushRemote({ forceWithLease: true })}>
                {m.git_header_push_force_lease()}
              </DropdownMenu.Item>
            {/if}
          </DropdownMenu.Content>
        </DropdownMenu.Root>
      {/if}

      {#if hasChanges}
        <Button
          class="size-7"
          aria-label={m.git_panel_mention_changes()}
          size="icon"
          title={m.git_panel_mention_changes()}
          variant="ghost"
          onclick={mentionChanges}
        >
          <AtSignIcon class="size-3.5" aria-hidden="true" />
        </Button>
      {/if}

      <Button
        class="size-7"
        aria-label={m.git_panel_refresh()}
        size="icon"
        title={m.git_panel_refresh()}
        variant="ghost"
        onclick={refreshAll}
      >
        <RefreshCwIcon class="size-3.5" aria-hidden="true" />
      </Button>
    </div>
  </div>

  <div class="flex flex-wrap items-center gap-x-2 gap-y-1 px-2 text-xs text-muted-foreground">
    {#if branch.upstream !== null}
      {#if isUpstreamGone}
        <Badge variant="outline">{m.git_refs_gone({ upstream: branch.upstream })}</Badge>
      {:else if isUpToDate}
        <span class="flex items-center gap-1" title={branch.upstream}>
          <CheckIcon class="size-3 text-git-added" aria-hidden="true" />
          <span>{m.git_header_up_to_date()}</span>
          <span class="font-mono text-muted-foreground">{branch.upstream}</span>
        </span>
      {:else}
        <span
          class="flex items-center gap-1.5 tabular-nums"
          title={m.git_refs_ahead_behind({
            ahead: formatCount(branch.ahead ?? 0),
            behind: formatCount(branch.behind ?? 0),
            upstream: branch.upstream,
          })}
        >
          <span class="font-mono">{branch.upstream}</span>
          {#if (branch.ahead ?? 0) > 0}
            <Badge class="h-4 px-1.5 text-[11px]" variant="secondary">
              {m.git_header_ahead({ count: formatCount(branch.ahead ?? 0) })}
            </Badge>
          {/if}
          {#if (branch.behind ?? 0) > 0}
            <Badge class="h-4 px-1.5 text-[11px]" variant="secondary">
              {m.git_header_behind({ count: formatCount(branch.behind ?? 0) })}
            </Badge>
          {/if}
        </span>
      {/if}
    {:else if canPushBranch}
      <span>{m.git_panel_no_upstream()}</span>
    {/if}

    {#if overview.stash > 0}
      <button
        class="cursor-pointer underline-offset-2 hover:text-foreground hover:underline"
        type="button"
        onclick={openStashes}
      >
        {m.git_panel_stash({ count: overview.stash })}
      </button>
    {/if}
  </div>
</header>
