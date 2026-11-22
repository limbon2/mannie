<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { Checkbox } from '@mannie/plugin-sdk/components/checkbox';
  import { Textarea } from '@mannie/plugin-sdk/components/textarea';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import ArchiveIcon from '@mannie/plugin-sdk/icons/archive';
  import GitCommitHorizontalIcon from '@mannie/plugin-sdk/icons/git-commit-horizontal';
  import { call } from '@mannie/plugin-sdk/ipc';

  import { commands, type Entry, type RepositoryOverview } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';
  import type { RepositoryPanel } from '../../state/git-repository.svelte.js';

  // Compact commit composer at the top of the Changes tab: message input
  // with Ctrl/Cmd+Enter shortcut, live subject character count, Amend toggle,
  // Stage-all-tracked toggle, Commit button, and Quick Stash button.
  interface Props {
    readonly overview: RepositoryOverview;
    readonly panel: RepositoryPanel;
  }

  let { overview, panel }: Props = $props();

  let message = $state('');
  let amend = $state(false);
  let stageAll = $state(false);
  let loadedAmendMessage = $state<string | null>(null);
  let loadingAmendMessage = $state(false);
  let amendLoadGeneration = 0;

  const firstLineLength = $derived(message.split('\n')[0]?.length ?? 0);
  const subjectLengthClass = $derived.by(() => {
    if (firstLineLength > 72) {
      return 'text-destructive';
    }
    if (firstLineLength > 50) {
      return 'text-git-modified';
    }
    return 'text-muted-foreground';
  });
  const hasStaged = $derived(overview.entries.some(isStagedEntry));
  const hasTrackedUnstaged = $derived(overview.entries.some(isTrackedUnstagedEntry));
  const hasAnyChanges = $derived(overview.entriesTotal > 0);
  const hasHead = $derived(overview.head !== null);
  const hasCommitContent = $derived.by(() => {
    const hasAutoStaged = stageAll && hasTrackedUnstaged;
    const hasTarget = hasStaged || hasAutoStaged;
    const hasMessage = message.trim().length > 0;
    return hasTarget && hasMessage;
  });
  const canCommit = $derived.by(() => {
    const isBusy = panel.runningAction || loadingAmendMessage;
    const isReady = amend ? hasHead : hasCommitContent;
    return !isBusy && isReady;
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

  function isTrackedUnstagedEntry(entry: Entry): boolean {
    const hasWorktreeChange = entry.unstaged !== '.';
    return isTrackedEntry(entry) && hasWorktreeChange;
  }

  function handleAmendChange(checked: boolean): void {
    amend = checked;
    amendLoadGeneration += 1;
    const generation = amendLoadGeneration;
    if (!checked) {
      loadingAmendMessage = false;
      const isUnchangedLoaded = loadedAmendMessage !== null && message.trimEnd() === loadedAmendMessage;
      if (isUnchangedLoaded) {
        message = '';
      }
      loadedAmendMessage = null;
      return;
    }

    loadedAmendMessage = null;
    const { head } = overview;
    const isMissingHead = head === null;
    const hasTypedMessage = message.trim().length > 0;
    if (isMissingHead || hasTypedMessage) {
      return;
    }

    loadingAmendMessage = true;
    void call(() => commands.repositoryCommit({ commit: head.oid, projectId: panel.projectId })).then((outcome) => {
      if (amendLoadGeneration !== generation) {
        return;
      }
      loadingAmendMessage = false;
      const canFill = outcome.ok && amend && message.trim().length === 0;
      if (canFill) {
        const fullMessage = outcome.data.commit.message.trimEnd();
        loadedAmendMessage = fullMessage;
        message = fullMessage;
      }
    });
  }

  async function submitCommit(): Promise<void> {
    if (!canCommit) {
      return;
    }
    const trimmed = message.trim();
    const isBlank = trimmed.length === 0;
    const isUnchangedAmend = loadedAmendMessage !== null && message.trimEnd() === loadedAmendMessage;
    const keepsOriginalOnAmend = amend && (isBlank || isUnchangedAmend);
    let replacementMessage: string | null = null;
    if (!keepsOriginalOnAmend && !isBlank) {
      replacementMessage = trimmed;
    }
    const result = await panel.runAction({
      all: stageAll,
      amend,
      kind: 'commit',
      message: replacementMessage,
    });
    if (result?.outcome === 'completed') {
      message = '';
      amend = false;
      stageAll = false;
      loadedAmendMessage = null;
    }
  }

  function handleKeydown(event: KeyboardEvent): void {
    const hasModifier = event.ctrlKey || event.metaKey;
    const isEnter = event.key === 'Enter';
    if (hasModifier && isEnter) {
      event.preventDefault();
      void submitCommit();
    }
  }

  function openStashDialog(): void {
    panel.openDialog({ kind: 'stash_push' });
  }
</script>

<div class="flex flex-col gap-2 border-b p-2.5" data-git-commit-box="">
  <div class="relative">
    <Textarea
      class="min-h-16 resize-y font-sans text-xs"
      aria-label={m.git_changes_commit_placeholder()}
      disabled={panel.runningAction}
      placeholder={m.git_changes_commit_placeholder()}
      rows={2}
      bind:value={message}
      onkeydown={handleKeydown}
    />
    {#if firstLineLength > 0}
      <span class={['pointer-events-none absolute inset-e-2 bottom-1.5 text-[10px] tabular-nums', subjectLengthClass]}>
        {m.git_changes_subject_length({ count: formatCount(firstLineLength) })}
      </span>
    {/if}
  </div>

  <div class="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
    {#if hasHead}
      <label class="flex cursor-pointer items-center gap-1.5">
        <Checkbox checked={amend} onCheckedChange={handleAmendChange} />
        <span>{m.git_changes_amend_label()}</span>
      </label>
    {/if}

    <label class="flex cursor-pointer items-center gap-1.5">
      <Checkbox bind:checked={stageAll} />
      <span>{m.git_changes_stage_all_label()}</span>
    </label>
  </div>

  <div class="flex items-center gap-1.5">
    <Button class="flex-1" disabled={!canCommit} size="sm" onclick={submitCommit}>
      <GitCommitHorizontalIcon class="size-3.5" aria-hidden="true" />
      <span>{amend ? m.git_changes_amend_button() : m.git_changes_commit_button()}</span>
    </Button>

    <Button
      disabled={panel.runningAction || !hasAnyChanges}
      size="sm"
      title={m.git_changes_stash_button()}
      variant="outline"
      onclick={openStashDialog}
    >
      <ArchiveIcon class="size-3.5" aria-hidden="true" />
      <span>{m.git_changes_stash_button()}</span>
    </Button>
  </div>
</div>
