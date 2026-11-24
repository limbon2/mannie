<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import * as Tabs from '@mannie/plugin-sdk/components/tabs';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import GitBranchIcon from '@mannie/plugin-sdk/icons/git-branch';
  import XIcon from '@mannie/plugin-sdk/icons/x';
  import type { SidePanelProps } from '@mannie/plugin-sdk/slots';
  import { untrack } from 'svelte';

  import type { OperationAction, OwnerGitAction } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { type GitPanelTab, RepositoryPanel } from '../state/git-repository.svelte.js';
  import type { GitFailure } from '../types/git.js';
  import { gitFailureMessage } from '../utils/git-messages.js';

  import BranchesTab from './panel/BranchesTab.svelte';
  import ChangesTab from './panel/ChangesTab.svelte';
  import CompareTab from './panel/CompareTab.svelte';
  import ConfirmActionDialog from './panel/ConfirmActionDialog.svelte';
  import ConflictsTab from './panel/ConflictsTab.svelte';
  import GitDialogs from './panel/GitDialogs.svelte';
  import GitPanelHeader from './panel/GitPanelHeader.svelte';
  import GraphTab from './panel/GraphTab.svelte';
  import InspectTab from './panel/InspectTab.svelte';
  import OperationBanner from './panel/OperationBanner.svelte';

  // The interactive Git side panel (plugins/git/docs/panel.md; ADR-0038,
  // ADR-0078). Registered with `hideHeading: true` so the panel header
  // carries the repository context without duplicating the tab label.
  let { argument, setLabel }: SidePanelProps = $props();

  let container = $state<HTMLElement | null>(null);
  let width = $state(0);

  const panel = $derived(new RepositoryPanel(argument));
  const wide = $derived(width >= 680);
  const overview = $derived(panel.overview.value);
  const conflictedCount = $derived(
    overview === null ? 0 : overview.entries.filter((entry) => entry.kind === 'conflicted').length,
  );
  const hasConflictsTab = $derived.by(() => {
    if (overview === null) {
      return false;
    }
    const hasConflicted = conflictedCount > 0;
    const hasOperation = overview.operation !== null;
    return hasConflicted || hasOperation;
  });
  const initialFailure = $derived.by(() => {
    const isEmpty = overview === null;
    const isError = panel.overview.status === 'error';
    if (isEmpty && isError) {
      return panel.overview.failure;
    }
    return null;
  });
  const refreshFailure = $derived.by(() => {
    const hasOverview = overview !== null;
    const isError = panel.overview.status === 'error';
    if (hasOverview && isError) {
      return panel.overview.failure;
    }
    return null;
  });
  const noticeDetail = $derived.by(() => {
    const detail = panel.actionNotice?.detail;
    if (detail === undefined) {
      return null;
    }
    return detail.length > 0 ? detail : null;
  });

  function repositoryName(top: string): string {
    const trimmed = top.replace(/[\\/]+$/, '');
    const parts = trimmed.split(/[\\/]/);
    const last = parts.at(-1);
    if (last === undefined) {
      return top;
    }
    return last.length > 0 ? last : top;
  }

  function isNotRepositoryFailure(failure: GitFailure): boolean {
    if (failure.kind !== 'core') {
      return false;
    }
    const isPlugin = failure.error.origin === 'plugin';
    const isNotRepo = failure.error.code === 'not_a_repository';
    return isPlugin && isNotRepo;
  }

  function canRetry(failure: GitFailure | null): boolean {
    if (failure === null) {
      return true;
    }
    return !isNotRepositoryFailure(failure);
  }

  function handleTabChange(next: string): void {
    const tab = next as GitPanelTab;
    panel.activeTab = tab;
    if (tab === 'graph') {
      if (panel.graphPage.status === 'idle') {
        void panel.readGraph(true);
      }
      return;
    }
    if (tab === 'branches') {
      if (panel.refs.status === 'idle') {
        void panel.readRefs();
      }
      return;
    }
    if (tab === 'compare') {
      if (panel.refs.status === 'idle') {
        void panel.readRefs();
      }
      if (panel.compareBase.length === 0) {
        if (overview !== null) {
          panel.compareBase = overview.branch.upstream ?? overview.branch.head ?? 'HEAD';
        }
      }
      return;
    }
    if (tab === 'inspect') {
      if (panel.fileTree.status === 'idle') {
        void panel.readFileTree();
      }
      return;
    }
    if (tab === 'conflicts') {
      if (panel.conflicts.status === 'idle') {
        void panel.readConflicts();
      }
    }
  }

  function handleOperationControl(action: OperationAction): void {
    void panel.runAction({ action, kind: 'operation_control' });
  }

  function handleDialogSubmit(action: OwnerGitAction): void {
    void panel.runAction(action);
  }

  function handleConfirmAction(): void {
    void panel.confirmPendingAction();
  }

  function retryOverview(): void {
    void panel.refresh();
  }

  $effect(() => {
    const current = panel;
    return untrack(() => current.watch());
  });

  $effect(() => {
    const currentOverview = overview;
    if (currentOverview !== null) {
      setLabel(repositoryName(currentOverview.top), true);
    }
  });

  $effect(() => {
    const isOnConflicts = panel.activeTab === 'conflicts';
    if (!hasConflictsTab && isOnConflicts) {
      panel.activeTab = 'changes';
    }
  });

  $effect(() => {
    const el = container;
    const isMissingEl = el === null;
    const hasNoObserver = typeof ResizeObserver === 'undefined';
    if (isMissingEl || hasNoObserver) {
      return;
    }
    const observer = new ResizeObserver((entries) => {
      const entry = entries.at(0);
      if (entry !== undefined) {
        width = entry.contentRect.width;
      }
    });
    observer.observe(el);
    return () => {
      observer.disconnect();
    };
  });
</script>

<div
  bind:this={container}
  class="flex h-full min-h-0 flex-col overflow-hidden bg-background text-foreground"
  data-git-panel=""
>
  {#if initialFailure !== null}
    <div class="flex flex-1 items-center justify-center p-4">
      <Empty.Root>
        <Empty.Header>
          <Empty.Media variant="icon">
            <GitBranchIcon aria-hidden="true" />
          </Empty.Media>
          <Empty.Title>{m.git_panel_failed_title()}</Empty.Title>
          <Empty.Description>{gitFailureMessage(initialFailure)}</Empty.Description>
        </Empty.Header>
        {#if canRetry(initialFailure)}
          <Empty.Content>
            <Button size="sm" variant="outline" onclick={retryOverview}>{m.git_panel_try_again()}</Button>
          </Empty.Content>
        {/if}
      </Empty.Root>
    </div>
  {:else if overview === null}
    <div class="flex flex-1 items-center justify-center gap-2 p-4 text-sm text-muted-foreground" aria-busy="true">
      <Spinner aria-label={m.git_panel_loading()} />
      <span>{m.git_panel_loading()}</span>
    </div>
  {:else}
    <GitPanelHeader {overview} {panel} />

    {#if overview.operation !== null}
      <OperationBanner busy={panel.runningAction} operation={overview.operation} onControl={handleOperationControl} />
    {/if}

    {#if refreshFailure !== null}
      <div class="border-b px-3 py-2">
        <Alert.Root variant="destructive">
          <Alert.Description class="flex items-center justify-between gap-2 text-xs">
            <span>{gitFailureMessage(refreshFailure)}</span>
            <Button size="sm" variant="outline" onclick={retryOverview}>{m.git_panel_try_again()}</Button>
          </Alert.Description>
        </Alert.Root>
      </div>
    {/if}

    {#if panel.actionNotice !== null}
      <div class="border-b px-3 py-2">
        <Alert.Root variant={panel.actionNotice.kind === 'error' ? 'destructive' : 'default'}>
          <Alert.Description class="flex items-start justify-between gap-2 text-xs">
            <div class="flex min-w-0 flex-1 flex-col gap-1">
              <span>{panel.actionNotice.message}</span>
              {#if noticeDetail !== null}
                <pre
                  class="max-h-24 overflow-auto rounded-xs bg-muted/60 p-1.5 font-mono text-[11px] whitespace-pre-wrap">{noticeDetail}</pre>
              {/if}
            </div>
            <Button
              class="size-5 shrink-0"
              aria-label={m.git_action_dismiss()}
              size="icon"
              variant="ghost"
              onclick={() => panel.dismissNotice()}
            >
              <XIcon class="size-3" aria-hidden="true" />
            </Button>
          </Alert.Description>
        </Alert.Root>
      </div>
    {/if}

    <Tabs.Root
      class="flex min-h-0 flex-1 flex-col overflow-hidden"
      value={panel.activeTab}
      onValueChange={handleTabChange}
    >
      <div class="flex items-center justify-between gap-2 overflow-x-auto border-b px-2.5">
        <Tabs.List class="h-8" aria-label={m.git_panel_tabs_label()} variant="line">
          <Tabs.Trigger class="gap-1.5 text-xs" value="changes">
            <span>{m.git_panel_changes()}</span>
            {#if overview.entriesTotal > 0}
              <Badge class="h-4 px-1 text-[10px]" variant="secondary">{formatCount(overview.entriesTotal)}</Badge>
            {/if}
          </Tabs.Trigger>

          <Tabs.Trigger class="text-xs" value="graph">{m.git_panel_graph()}</Tabs.Trigger>

          <Tabs.Trigger class="text-xs" value="branches">{m.git_panel_branches()}</Tabs.Trigger>

          <Tabs.Trigger class="text-xs" value="compare">{m.git_panel_compare()}</Tabs.Trigger>

          <Tabs.Trigger class="text-xs" value="inspect">{m.git_panel_inspect()}</Tabs.Trigger>

          {#if hasConflictsTab}
            <Tabs.Trigger class="gap-1.5 text-xs" value="conflicts">
              <span>{m.git_panel_conflicts()}</span>
              {#if conflictedCount > 0}
                <Badge class="h-4 px-1 text-[10px]" variant="destructive">{formatCount(conflictedCount)}</Badge>
              {/if}
            </Tabs.Trigger>
          {/if}
        </Tabs.List>

        {#if panel.runningAction}
          <div class="flex shrink-0 items-center gap-1.5 text-xs text-muted-foreground" aria-busy="true">
            <Spinner class="size-3.5" aria-label={m.git_action_running()} />
          </div>
        {/if}
      </div>

      <Tabs.Content class="flex min-h-0 flex-1 flex-col overflow-hidden" value="changes">
        <ChangesTab {overview} {panel} {wide} />
      </Tabs.Content>

      <Tabs.Content class="flex min-h-0 flex-1 flex-col overflow-hidden" value="graph">
        <GraphTab {panel} {wide} />
      </Tabs.Content>

      <Tabs.Content class="flex min-h-0 flex-1 flex-col overflow-hidden" value="branches">
        <BranchesTab {panel} {wide} />
      </Tabs.Content>

      <Tabs.Content class="flex min-h-0 flex-1 flex-col overflow-hidden" value="compare">
        <CompareTab {overview} {panel} />
      </Tabs.Content>

      <Tabs.Content class="flex min-h-0 flex-1 flex-col overflow-hidden" value="inspect">
        <InspectTab {panel} />
      </Tabs.Content>

      {#if hasConflictsTab}
        <Tabs.Content class="flex min-h-0 flex-1 flex-col overflow-hidden" value="conflicts">
          <ConflictsTab {panel} />
        </Tabs.Content>
      {/if}
    </Tabs.Root>

    <ConfirmActionDialog
      busy={panel.runningAction}
      pending={panel.pendingConfirmation}
      onCancel={() => panel.cancelPendingAction()}
      onConfirm={handleConfirmAction}
    />

    <GitDialogs
      busy={panel.runningAction}
      dialog={panel.activeDialog}
      onClose={() => panel.closeDialog()}
      onSubmit={handleDialogSubmit}
    />
  {/if}
</div>
