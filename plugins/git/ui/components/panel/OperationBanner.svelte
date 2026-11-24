<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import PlayIcon from '@mannie/plugin-sdk/icons/play';
  import SkipForwardIcon from '@mannie/plugin-sdk/icons/skip-forward';
  import TriangleAlertIcon from '@mannie/plugin-sdk/icons/triangle-alert';
  import XIcon from '@mannie/plugin-sdk/icons/x';

  import type { OperationAction, OperationView } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';

  // High-visibility banner shown while a merge, rebase, cherry-pick or revert
  // is paused in progress, offering Continue, Skip and Abort controls.
  interface Props {
    readonly busy: boolean;
    readonly onControl: (action: OperationAction) => void;
    readonly operation: OperationView;
  }

  let { busy, onControl, operation }: Props = $props();

  const summary = $derived.by(() => {
    switch (operation.kind) {
      case 'cherry_pick': {
        return m.git_panel_operation_cherry_pick();
      }
      case 'merge': {
        return m.git_panel_operation_merge();
      }
      case 'rebase': {
        return operation.step === null
          ? m.git_panel_operation_rebase()
          : m.git_panel_operation_rebase_step({
              step: formatCount(operation.step.number),
              total: formatCount(operation.step.total),
            });
      }
      case 'revert': {
        return m.git_panel_operation_revert();
      }
    }
  });
</script>

<div
  class="flex flex-wrap items-center justify-between gap-2 border-b bg-muted/60 px-3 py-2 text-xs"
  data-git-operation-banner=""
>
  <div class="flex min-w-0 items-center gap-2">
    <TriangleAlertIcon class="size-4 shrink-0 text-git-conflicted" aria-hidden="true" />
    <div class="flex min-w-0 flex-col">
      <span class="font-medium text-foreground">{summary}</span>
      {#if operation.incoming !== null}
        <span class="truncate text-muted-foreground">
          <span class="me-1 font-mono">{operation.incoming.short}</span>
          {operation.incoming.subject}
        </span>
      {/if}
    </div>
  </div>

  <div class="flex shrink-0 items-center gap-1">
    <Button disabled={busy} size="sm" variant="default" onclick={() => onControl('continue')}>
      <PlayIcon class="size-3.5" aria-hidden="true" />
      <span>{m.git_operation_continue()}</span>
    </Button>

    {#if operation.kind !== 'merge'}
      <Button disabled={busy} size="sm" variant="outline" onclick={() => onControl('skip')}>
        <SkipForwardIcon class="size-3.5" aria-hidden="true" />
        <span>{m.git_operation_skip()}</span>
      </Button>
    {/if}

    <Button disabled={busy} size="sm" variant="destructive" onclick={() => onControl('abort')}>
      <XIcon class="size-3.5" aria-hidden="true" />
      <span>{m.git_operation_abort()}</span>
    </Button>
  </div>
</div>
