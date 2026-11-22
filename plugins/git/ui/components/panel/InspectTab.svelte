<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { Checkbox } from '@mannie/plugin-sdk/components/checkbox';
  import * as Command from '@mannie/plugin-sdk/components/command';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import * as Popover from '@mannie/plugin-sdk/components/popover';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import * as Tabs from '@mannie/plugin-sdk/components/tabs';
  import AtSignIcon from '@mannie/plugin-sdk/icons/at-sign';
  import ChevronDownIcon from '@mannie/plugin-sdk/icons/chevron-down';
  import FileClockIcon from '@mannie/plugin-sdk/icons/file-clock';
  import FileCodeIcon from '@mannie/plugin-sdk/icons/file-code';
  import GitCommitHorizontalIcon from '@mannie/plugin-sdk/icons/git-commit-horizontal';
  import ScanSearchIcon from '@mannie/plugin-sdk/icons/scan-search';
  import SearchIcon from '@mannie/plugin-sdk/icons/search';
  import { SvelteMap } from 'svelte/reactivity';

  import type { BlameRunView, Match } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';
  import type { RepositoryPanel } from '../../state/git-repository.svelte.js';
  import { formatFullDate, formatRelativeTime, heatLevel, mentionEntryForCommit } from '../../utils/git-format.js';
  import { gitFailureMessage } from '../../utils/git-messages.js';
  import CommitRow from '../cards/CommitRow.svelte';

  // Inspect tab: interactive Blame with age heat bar and commit popover,
  // per-file `--follow` commit history, and repository-wide `git grep` code
  // search jumping straight to Blame or Commit Inspector.
  interface Props {
    readonly panel: RepositoryPanel;
  }

  interface GroupedMatches {
    readonly matches: readonly Match[];
    readonly path: string;
  }

  let { panel }: Props = $props();

  let filePickerOpen = $state(false);
  let treeSearch = $state('');
  const filePickerListId = 'git-inspect-file-picker-list';

  const HEAT_CLASSES: readonly string[] = [
    'bg-git-heat-0',
    'bg-git-heat-1',
    'bg-git-heat-2',
    'bg-git-heat-3',
    'bg-git-heat-4',
  ];

  const blameView = $derived(panel.blame.value);
  const isBlameLoading = $derived(panel.blame.status === 'loading' && blameView === null);
  const blameFailure = $derived(panel.blame.status === 'error' ? panel.blame.failure : null);
  const fileHistoryView = $derived(panel.fileHistory.value);
  const isHistoryLoading = $derived(panel.fileHistory.status === 'loading' && fileHistoryView === null);
  const historyFailure = $derived(panel.fileHistory.status === 'error' ? panel.fileHistory.failure : null);
  const grepView = $derived(panel.grepResult.value);
  const isGrepLoading = $derived(panel.grepResult.status === 'loading' && grepView === null);
  const grepFailure = $derived(panel.grepResult.status === 'error' ? panel.grepResult.failure : null);
  const groupedMatches: readonly GroupedMatches[] = $derived.by(() => {
    const groups = new SvelteMap<string, Match[]>();
    for (const found of grepView?.matches ?? []) {
      const list = groups.get(found.path);
      if (list === undefined) {
        groups.set(found.path, [found]);
      } else {
        list.push(found);
      }
    }
    return [...groups.entries()].map(([path, matches]) => ({ matches, path }));
  });

  function heatClass(run: BlameRunView): string {
    const isUncommitted = run.commit.uncommitted;
    const isMissingView = blameView === null;
    if (isUncommitted || isMissingView) {
      return 'bg-git-untracked';
    }
    const bucket = heatLevel(run.commit.timestamp, blameView.minTimestamp, blameView.maxTimestamp);
    return HEAT_CLASSES[bucket] ?? 'bg-git-heat-2';
  }

  function handlePickerOpenChange(open: boolean): void {
    filePickerOpen = open;
  }

  function chooseInspectFile(path: string): void {
    filePickerOpen = false;
    panel.inspectPath = path;
    if (panel.inspectMode === 'blame') {
      void panel.readBlame();
    } else if (panel.inspectMode === 'history') {
      void panel.readFileHistory();
    }
  }

  function handleModeChange(next: string): void {
    const isBlame = next === 'blame';
    const isHistory = next === 'history';
    const isGrep = next === 'grep';
    if (!isBlame && !isHistory && !isGrep) {
      return;
    }
    panel.inspectMode = next;
    if (panel.inspectPath.length === 0) {
      return;
    }
    if (isBlame) {
      if (panel.blame.status === 'idle') {
        void panel.readBlame();
      }
    } else if (isHistory) {
      if (panel.fileHistory.status === 'idle') {
        void panel.readFileHistory();
      }
    }
  }

  function handleIgnoreWsChange(checked: boolean): void {
    panel.blameIgnoreWhitespace = checked;
    void panel.readBlame();
  }

  function handleFollowMovesChange(checked: boolean): void {
    panel.blameFollowMoves = checked;
    void panel.readBlame();
  }

  function submitGrep(event: SubmitEvent): void {
    event.preventDefault();
    void panel.runGrep();
  }

  function submitRevision(event: SubmitEvent): void {
    event.preventDefault();
    if (panel.inspectMode === 'blame') {
      void panel.readBlame();
    } else if (panel.inspectMode === 'grep') {
      void panel.runGrep();
    }
  }

  function mentionBlameRun(run: BlameRunView): void {
    panel.mentionInChat(
      mentionEntryForCommit({
        oid: run.commit.oid,
        short: run.short,
        subject: run.commit.summary,
      }),
    );
  }

  $effect(() => {
    if (!filePickerOpen) {
      return;
    }
    const query = treeSearch;
    const timer = setTimeout(() => {
      void panel.readFileTree(query);
    }, 100);
    return () => {
      clearTimeout(timer);
    };
  });
</script>

<div class="flex h-full min-h-0 flex-col overflow-hidden">
  <Tabs.Root
    class="flex min-h-0 flex-1 flex-col overflow-hidden"
    value={panel.inspectMode}
    onValueChange={handleModeChange}
  >
    <div class="border-b px-2.5 pt-1.5">
      <Tabs.List variant="line">
        <Tabs.Trigger class="gap-1 text-xs" value="blame">
          <ScanSearchIcon class="size-3.5" aria-hidden="true" />
          <span>{m.git_inspect_blame()}</span>
        </Tabs.Trigger>
        <Tabs.Trigger class="gap-1 text-xs" value="history">
          <FileClockIcon class="size-3.5" aria-hidden="true" />
          <span>{m.git_inspect_file_history()}</span>
        </Tabs.Trigger>
        <Tabs.Trigger class="gap-1 text-xs" value="grep">
          <SearchIcon class="size-3.5" aria-hidden="true" />
          <span>{m.git_inspect_search()}</span>
        </Tabs.Trigger>
      </Tabs.List>
    </div>

    {#if panel.inspectMode !== 'grep'}
      <div class="flex flex-wrap items-center gap-1.5 border-b px-2.5 py-2">
        <Popover.Root open={filePickerOpen} onOpenChange={handlePickerOpenChange}>
          <Popover.Trigger>
            {#snippet child({ props })}
              <Button
                {...props}
                class="h-7 min-w-40 flex-1 justify-between gap-1.5 px-2 font-mono text-xs"
                size="sm"
                variant="outline"
              >
                <span class="flex min-w-0 items-center gap-1.5">
                  <FileCodeIcon class="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
                  <span class="truncate">{panel.inspectPath || m.git_inspect_choose_file()}</span>
                </span>
                <ChevronDownIcon class="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
              </Button>
            {/snippet}
          </Popover.Trigger>
          <Popover.Content class="w-80 p-0" align="start">
            <Command.Root shouldFilter={false}>
              <Command.Input
                aria-controls={filePickerListId}
                placeholder={m.git_inspect_search_files()}
                bind:value={treeSearch}
              />
              <Command.List id={filePickerListId} aria-label={m.git_inspect_choose_file()}>
                <Command.Empty>{m.git_inspect_no_files()}</Command.Empty>
                <Command.Group>
                  {#each panel.fileTree.value?.files ?? [] as filePath (filePath)}
                    <Command.Item value={filePath} onSelect={() => chooseInspectFile(filePath)}>
                      <span class="truncate font-mono text-xs">{filePath}</span>
                    </Command.Item>
                  {/each}
                </Command.Group>
              </Command.List>
            </Command.Root>
          </Popover.Content>
        </Popover.Root>

        {#if panel.inspectMode === 'blame'}
          <form class="flex items-center gap-1" onsubmit={submitRevision}>
            <Input
              class="h-7 w-28 font-mono text-xs"
              aria-label={m.git_inspect_revision_worktree()}
              placeholder={m.git_inspect_revision_worktree()}
              bind:value={panel.inspectRevision}
            />
          </form>

          <label class="flex cursor-pointer items-center gap-1 text-xs text-muted-foreground">
            <Checkbox checked={panel.blameIgnoreWhitespace} onCheckedChange={handleIgnoreWsChange} />
            <span>{m.git_diff_ignore_whitespace()}</span>
          </label>

          <label class="flex cursor-pointer items-center gap-1 text-xs text-muted-foreground">
            <Checkbox checked={panel.blameFollowMoves} onCheckedChange={handleFollowMovesChange} />
            <span>{m.git_inspect_follow_moves()}</span>
          </label>
        {/if}
      </div>
    {/if}

    <!-- Blame Tab -->
    <Tabs.Content class="min-h-0 flex-1 overflow-auto" value="blame">
      {#if isBlameLoading}
        <div class="flex items-center gap-2 p-3 text-xs text-muted-foreground" aria-busy="true">
          <Spinner aria-label={m.git_panel_loading()} />
          <span>{m.git_panel_loading()}</span>
        </div>
      {:else if blameFailure !== null}
        <div class="p-3">
          <Alert.Root variant="destructive">
            <Alert.Description class="flex items-center justify-between gap-2 text-xs">
              <span>{gitFailureMessage(blameFailure)}</span>
              <Button size="sm" variant="outline" onclick={() => panel.readBlame()}>{m.git_panel_try_again()}</Button>
            </Alert.Description>
          </Alert.Root>
        </div>
      {:else if blameView === null}
        <p class="p-4 text-center text-xs text-muted-foreground">{m.git_inspect_blame_empty()}</p>
      {:else if blameView.binary}
        <p class="p-4 text-center text-xs text-muted-foreground">{m.git_diff_binary()}</p>
      {:else}
        {#if blameView.truncated}
          <p class="border-b bg-muted/40 px-2.5 py-1 text-xs text-muted-foreground">
            {m.git_inspect_blame_truncated()}
          </p>
        {/if}

        <div class="font-mono text-xs/5" data-git-blame="">
          {#each blameView.runs as run, runIndex (`${run.commit.oid}-${runIndex}`)}
            <div class="border-b border-border/50 last:border-b-0">
              {#each run.lines as line, lineIndex (line.number)}
                <div class="flex min-w-max items-stretch hover:bg-muted/40">
                  <div class={['w-1 shrink-0', heatClass(run)]} aria-hidden="true"></div>

                  <div class="flex w-44 shrink-0 items-center border-e bg-muted/20 px-1.5 font-sans text-[11px]">
                    {#if lineIndex === 0}
                      {#if run.commit.uncommitted}
                        <span class="truncate font-medium text-git-untracked">{m.git_inspect_blame_uncommitted()}</span>
                      {:else}
                        <Popover.Root>
                          <Popover.Trigger>
                            {#snippet child({ props })}
                              <button
                                {...props}
                                class="flex w-full cursor-pointer items-center gap-1.5 text-start hover:text-foreground"
                                type="button"
                              >
                                <span class="shrink-0 font-mono text-[10px] text-muted-foreground">{run.short}</span>
                                <span class="min-w-0 flex-1 truncate text-foreground">{run.commit.author}</span>
                                {#if run.commit.authored.length > 0}
                                  <span class="shrink-0 text-[10px] text-muted-foreground tabular-nums">
                                    {formatRelativeTime(run.commit.authored)}
                                  </span>
                                {/if}
                              </button>
                            {/snippet}
                          </Popover.Trigger>
                          <Popover.Content class="w-80 p-3 font-sans text-xs" align="start">
                            <div class="flex flex-col gap-2">
                              <div class="flex items-center gap-1.5">
                                <Badge class="font-mono" variant="secondary">{run.short}</Badge>
                                <span class="truncate font-medium">{run.commit.author}</span>
                                {#if run.commit.authored.length > 0}
                                  <span
                                    class="ms-auto shrink-0 text-muted-foreground"
                                    title={formatFullDate(run.commit.authored)}
                                  >
                                    {formatRelativeTime(run.commit.authored)}
                                  </span>
                                {/if}
                              </div>
                              <p class="font-medium text-foreground">{run.commit.summary}</p>
                              <div class="flex flex-wrap items-center gap-1 pt-1">
                                <Button
                                  class="h-6 px-2 text-xs"
                                  size="sm"
                                  variant="outline"
                                  onclick={() => panel.openCommitInGraph(run.commit.oid)}
                                >
                                  <GitCommitHorizontalIcon class="size-3" aria-hidden="true" />
                                  <span>{m.git_inspect_inspect_commit()}</span>
                                </Button>
                                <Button
                                  class="h-6 px-2 text-xs"
                                  size="sm"
                                  variant="ghost"
                                  onclick={() => mentionBlameRun(run)}
                                >
                                  <AtSignIcon class="size-3" aria-hidden="true" />
                                  <span>{m.git_panel_mention_commit()}</span>
                                </Button>
                              </div>
                            </div>
                          </Popover.Content>
                        </Popover.Root>
                      {/if}
                    {/if}
                  </div>

                  <span class="w-10 shrink-0 pe-2 text-end text-muted-foreground select-none" aria-hidden="true">
                    {line.number}
                  </span>
                  <span class="pe-3 whitespace-pre">{line.text}</span>
                </div>
              {/each}
            </div>
          {/each}
        </div>
      {/if}
    </Tabs.Content>

    <!-- File History Tab -->
    <Tabs.Content class="min-h-0 flex-1 overflow-y-auto p-2.5" value="history">
      {#if isHistoryLoading}
        <div class="flex items-center gap-2 py-4 text-xs text-muted-foreground" aria-busy="true">
          <Spinner aria-label={m.git_panel_loading()} />
          <span>{m.git_panel_loading()}</span>
        </div>
      {:else if historyFailure !== null}
        <Alert.Root variant="destructive">
          <Alert.Description class="flex items-center justify-between gap-2 text-xs">
            <span>{gitFailureMessage(historyFailure)}</span>
            <Button size="sm" variant="outline" onclick={() => panel.readFileHistory()}>
              {m.git_panel_try_again()}
            </Button>
          </Alert.Description>
        </Alert.Root>
      {:else if fileHistoryView === null}
        <p class="py-4 text-center text-xs text-muted-foreground">{m.git_inspect_file_history_empty()}</p>
      {:else if fileHistoryView.commits.length === 0}
        <p class="py-4 text-center text-xs text-muted-foreground">{m.git_graph_no_matches()}</p>
      {:else}
        <ul class="flex flex-col divide-y rounded-md border">
          {#each fileHistoryView.commits as commitItem (commitItem.oid)}
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

    <!-- Code Search (git grep) Tab -->
    <Tabs.Content class="flex min-h-0 flex-1 flex-col overflow-hidden" value="grep">
      <form class="flex flex-col gap-1.5 border-b p-2.5" onsubmit={submitGrep}>
        <div class="flex items-center gap-1.5">
          <div class="relative flex-1">
            <SearchIcon
              class="pointer-events-none absolute inset-s-2 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground"
              aria-hidden="true"
            />
            <Input
              class="h-7 ps-7 text-xs"
              aria-label={m.git_inspect_grep_placeholder()}
              placeholder={m.git_inspect_grep_placeholder()}
              bind:value={panel.grepPattern}
            />
          </div>
          <Button class="h-7 px-2.5 text-xs" size="sm" type="submit">{m.git_inspect_grep_submit()}</Button>
        </div>

        <div class="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <Input
            class="h-7 min-w-32 flex-1 font-mono text-xs"
            aria-label={m.git_inspect_grep_path_placeholder()}
            placeholder={m.git_inspect_grep_path_placeholder()}
            bind:value={panel.grepPath}
          />
          <label class="flex cursor-pointer items-center gap-1">
            <Checkbox bind:checked={panel.grepCaseSensitive} />
            <span>{m.git_inspect_grep_case_sensitive()}</span>
          </label>
          <label class="flex cursor-pointer items-center gap-1">
            <Checkbox bind:checked={panel.grepRegex} />
            <span>{m.git_inspect_grep_regex()}</span>
          </label>
        </div>
      </form>

      <div class="min-h-0 flex-1 overflow-y-auto p-2.5">
        {#if isGrepLoading}
          <div class="flex items-center gap-2 py-4 text-xs text-muted-foreground" aria-busy="true">
            <Spinner aria-label={m.git_panel_loading()} />
            <span>{m.git_panel_loading()}</span>
          </div>
        {:else if grepFailure !== null}
          <Alert.Root variant="destructive">
            <Alert.Description class="text-xs">{gitFailureMessage(grepFailure)}</Alert.Description>
          </Alert.Root>
        {:else if grepView === null}
          <p class="py-4 text-center text-xs text-muted-foreground">{m.git_inspect_grep_empty()}</p>
        {:else if groupedMatches.length === 0}
          <p class="py-4 text-center text-xs text-muted-foreground">{m.git_inspect_grep_no_matches()}</p>
        {:else}
          <div class="flex flex-col gap-2">
            {#if grepView.truncated}
              <p class="rounded-md border bg-muted/40 px-2.5 py-1 text-xs text-muted-foreground">
                {m.git_inspect_grep_truncated()}
              </p>
            {/if}

            {#each groupedMatches as group (group.path)}
              <div class="overflow-hidden rounded-md border">
                <div class="flex items-center justify-between gap-2 border-b bg-muted/40 px-2.5 py-1 text-xs">
                  <button
                    class="min-w-0 truncate font-mono font-medium hover:underline"
                    type="button"
                    onclick={() => panel.openFileBlame(group.path)}
                  >
                    {group.path}
                  </button>
                  <Badge class="h-4 px-1.5 text-[10px]" variant="secondary">{group.matches.length}</Badge>
                </div>

                <ul class="divide-y font-mono text-xs/5">
                  {#each group.matches as found (`${found.path}-${found.line}-${found.text}`)}
                    <li>
                      <button
                        class="flex w-full cursor-pointer items-baseline gap-2 px-2.5 py-1 text-start hover:bg-muted/50"
                        type="button"
                        onclick={() => panel.openFileBlame(found.path, found.commit ?? '')}
                      >
                        <span class="w-10 shrink-0 text-end text-muted-foreground tabular-nums">{found.line}</span>
                        <span class="min-w-0 truncate whitespace-pre">{found.text}</span>
                      </button>
                    </li>
                  {/each}
                </ul>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    </Tabs.Content>
  </Tabs.Root>
</div>
