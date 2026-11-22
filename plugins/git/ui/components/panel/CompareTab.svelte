<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import * as Select from '@mannie/plugin-sdk/components/select';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import * as Tabs from '@mannie/plugin-sdk/components/tabs';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import ArrowLeftRightIcon from '@mannie/plugin-sdk/icons/arrow-left-right';
  import Columns2Icon from '@mannie/plugin-sdk/icons/columns-2';
  import GitCompareIcon from '@mannie/plugin-sdk/icons/git-compare';
  import Rows2Icon from '@mannie/plugin-sdk/icons/rows-2';
  import SpaceIcon from '@mannie/plugin-sdk/icons/space';
  import { SvelteSet } from 'svelte/reactivity';

  import type { RepositoryOverview } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';
  import type { RepositoryPanel } from '../../state/git-repository.svelte.js';
  import { gitFailureMessage } from '../../utils/git-messages.js';
  import CommitRow from '../cards/CommitRow.svelte';
  import DiffView from '../cards/DiffView.svelte';

  // Compare any two branches, tags, commits, or working tree (`base...head`
  // or `base..head`) with ahead/behind commit lists and full file diffs.
  interface Props {
    readonly overview: RepositoryOverview;
    readonly panel: RepositoryPanel;
  }

  let { overview, panel }: Props = $props();

  let subTab = $state<'ahead' | 'behind' | 'files'>('files');

  const refs = $derived(panel.refs.value);
  const result = $derived(panel.compareResult.value);
  const isLoadingInitial = $derived(panel.compareResult.status === 'loading' && result === null);
  const loadFailure = $derived(panel.compareResult.status === 'error' ? panel.compareResult.failure : null);
  const refChoices = $derived.by(() => {
    const names = new SvelteSet<string>(['HEAD']);
    if (overview.branch.head !== null) {
      names.add(overview.branch.head);
    }
    if (overview.branch.upstream !== null) {
      names.add(overview.branch.upstream);
    }
    for (const branch of refs?.branches ?? []) {
      names.add(branch.name);
    }
    for (const remote of refs?.remoteBranches ?? []) {
      names.add(remote.name);
    }
    for (const tagItem of refs?.tags.slice(0, 30) ?? []) {
      names.add(tagItem.name);
    }
    return [...names];
  });

  function submitCompare(event: SubmitEvent): void {
    event.preventDefault();
    void panel.runCompare();
  }

  function handleModeChange(next: string): void {
    const isTwoDot = next === 'two_dot';
    const isThreeDot = next === 'three_dot';
    if (isTwoDot || isThreeDot) {
      panel.compareMode = next;
      void panel.runCompare();
    }
  }

  function toggleDiffMode(): void {
    panel.diffMode = panel.diffMode === 'unified' ? 'split' : 'unified';
  }

  function compareUpstream(): void {
    if (overview.branch.upstream !== null) {
      panel.openCompare(overview.branch.upstream, 'HEAD');
    }
  }

  function compareWorktree(): void {
    panel.openCompare('HEAD', 'WORKTREE');
  }
</script>

<div class="flex h-full min-h-0 flex-col overflow-hidden">
  <form class="flex flex-col gap-2 border-b p-2.5" onsubmit={submitCompare}>
    <div class="grid grid-cols-[1fr_auto_1fr] items-end gap-1.5">
      <div class="flex flex-col gap-1">
        <label class="text-[11px] font-medium text-muted-foreground" for="git-compare-base">
          {m.git_compare_base()}
        </label>
        <Input
          id="git-compare-base"
          class="h-7 font-mono text-xs"
          list="git-compare-ref-choices"
          placeholder={m.git_compare_base_placeholder()}
          bind:value={panel.compareBase}
        />
      </div>

      <Button
        class="size-7 shrink-0"
        aria-label={m.git_compare_swap()}
        disabled={panel.compareHead === 'WORKTREE'}
        size="icon"
        title={m.git_compare_swap()}
        type="button"
        variant="ghost"
        onclick={() => panel.swapCompare()}
      >
        <ArrowLeftRightIcon class="size-3.5" aria-hidden="true" />
      </Button>

      <div class="flex flex-col gap-1">
        <label class="text-[11px] font-medium text-muted-foreground" for="git-compare-head">
          {m.git_compare_head()}
        </label>
        <Input
          id="git-compare-head"
          class="h-7 font-mono text-xs"
          list="git-compare-head-choices"
          placeholder={m.git_placeholder_head()}
          bind:value={panel.compareHead}
        />
      </div>
    </div>

    <datalist id="git-compare-ref-choices">
      {#each refChoices as name (name)}
        <option value={name}></option>
      {/each}
    </datalist>

    <datalist id="git-compare-head-choices">
      <option value="WORKTREE">{m.git_compare_worktree()}</option>
      {#each refChoices as name (name)}
        <option value={name}></option>
      {/each}
    </datalist>

    <div class="flex flex-wrap items-center gap-1.5">
      <Select.Root type="single" value={panel.compareMode} onValueChange={handleModeChange}>
        <Select.Trigger class="h-7 w-44 text-xs">
          {panel.compareMode === 'two_dot' ? m.git_compare_two_dot() : m.git_compare_three_dot()}
        </Select.Trigger>
        <Select.Content>
          <Select.Item value="three_dot">{m.git_compare_three_dot()}</Select.Item>
          <Select.Item value="two_dot">{m.git_compare_two_dot()}</Select.Item>
        </Select.Content>
      </Select.Root>

      <Button class="h-7 px-2.5 text-xs" size="sm" type="submit">
        <GitCompareIcon class="size-3.5" aria-hidden="true" />
        <span>{m.git_compare_run()}</span>
      </Button>

      <div class="ms-auto flex items-center gap-0.5">
        <Button
          class="size-7"
          aria-label={panel.diffMode === 'unified' ? m.git_diff_split() : m.git_diff_unified()}
          aria-pressed={panel.diffMode === 'split'}
          size="icon"
          title={panel.diffMode === 'unified' ? m.git_diff_split() : m.git_diff_unified()}
          type="button"
          variant={panel.diffMode === 'split' ? 'secondary' : 'ghost'}
          onclick={toggleDiffMode}
        >
          {#if panel.diffMode === 'unified'}
            <Columns2Icon class="size-3.5" aria-hidden="true" />
          {:else}
            <Rows2Icon class="size-3.5" aria-hidden="true" />
          {/if}
        </Button>

        <Button
          class="size-7"
          aria-label={m.git_diff_ignore_whitespace()}
          aria-pressed={panel.ignoreWhitespace}
          size="icon"
          title={m.git_diff_ignore_whitespace()}
          type="button"
          variant={panel.ignoreWhitespace ? 'secondary' : 'ghost'}
          onclick={() => panel.toggleIgnoreWhitespace()}
        >
          <SpaceIcon class="size-3.5" aria-hidden="true" />
        </Button>
      </div>
    </div>

    <div class="flex flex-wrap items-center gap-1">
      {#if overview.branch.upstream !== null}
        <Button class="h-6 px-2 text-[11px]" size="sm" type="button" variant="outline" onclick={compareUpstream}>
          {m.git_compare_preset_upstream({ upstream: overview.branch.upstream })}
        </Button>
      {/if}
      <Button class="h-6 px-2 text-[11px]" size="sm" type="button" variant="outline" onclick={compareWorktree}>
        {m.git_compare_preset_worktree()}
      </Button>
    </div>
  </form>

  <div class="flex min-h-0 flex-1 flex-col overflow-hidden">
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
            <Button size="sm" variant="outline" onclick={() => panel.runCompare()}>{m.git_panel_try_again()}</Button>
          </Alert.Description>
        </Alert.Root>
      </div>
    {:else if result === null}
      <p class="p-4 text-center text-xs text-muted-foreground">{m.git_compare_prompt()}</p>
    {:else}
      <Tabs.Root class="flex min-h-0 flex-1 flex-col overflow-hidden" bind:value={subTab}>
        <Tabs.List class="mx-2.5 mt-1.5 w-fit" variant="line">
          <Tabs.Trigger class="text-xs" value="files">
            {m.git_compare_files_tab({ count: formatCount(result.files.length) })}
          </Tabs.Trigger>
          {#if result.head !== null}
            <Tabs.Trigger class="text-xs" value="ahead">
              {m.git_compare_ahead_tab({ count: formatCount(result.ahead.length) })}
            </Tabs.Trigger>
            <Tabs.Trigger class="text-xs" value="behind">
              {m.git_compare_behind_tab({ count: formatCount(result.behind.length) })}
            </Tabs.Trigger>
          {/if}
        </Tabs.List>

        <Tabs.Content class="min-h-0 flex-1 overflow-y-auto p-2.5" value="files">
          {#if result.files.length === 0}
            <p class="py-4 text-center text-xs text-muted-foreground">
              {m.git_compare_no_diff({ base: result.baseLabel, head: result.headLabel })}
            </p>
          {:else}
            <DiffView
              files={result.files}
              mode={panel.diffMode}
              open={result.files.length === 1}
              patches={result.patches}
              onOpenBlame={(path: string) => panel.openFileBlame(path, result.head ?? '')}
              onOpenHistory={(path: string) => panel.openFileHistory(path)}
            />
          {/if}
        </Tabs.Content>

        {#if result.head !== null}
          <Tabs.Content class="min-h-0 flex-1 overflow-y-auto p-2.5" value="ahead">
            {#if result.ahead.length === 0}
              <p class="py-4 text-center text-xs text-muted-foreground">{m.git_compare_no_commits()}</p>
            {:else}
              <ul class="flex flex-col divide-y rounded-md border">
                {#each result.ahead as commitItem (commitItem.oid)}
                  <li>
                    <button
                      class="flex w-full cursor-pointer items-center gap-2 px-2.5 py-1.5 text-start text-xs hover:bg-muted/70"
                      type="button"
                      onclick={() => panel.openCommitInGraph(commitItem.oid)}
                    >
                      <CommitRow commit={commitItem} />
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          </Tabs.Content>

          <Tabs.Content class="min-h-0 flex-1 overflow-y-auto p-2.5" value="behind">
            {#if result.behind.length === 0}
              <p class="py-4 text-center text-xs text-muted-foreground">{m.git_compare_no_commits()}</p>
            {:else}
              <ul class="flex flex-col divide-y rounded-md border">
                {#each result.behind as commitItem (commitItem.oid)}
                  <li>
                    <button
                      class="flex w-full cursor-pointer items-center gap-2 px-2.5 py-1.5 text-start text-xs hover:bg-muted/70"
                      type="button"
                      onclick={() => panel.openCommitInGraph(commitItem.oid)}
                    >
                      <CommitRow commit={commitItem} />
                    </button>
                  </li>
                {/each}
              </ul>
            {/if}
          </Tabs.Content>
        {/if}
      </Tabs.Root>
    {/if}
  </div>
</div>
