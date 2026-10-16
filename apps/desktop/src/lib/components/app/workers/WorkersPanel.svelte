<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import PlusIcon from '@lucide/svelte/icons/plus';
  import { onMount } from 'svelte';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as Sidebar from '#lib/components/ui/sidebar/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { workersActions } from '#lib/state/workers-actions.svelte.js';
  import { workerList } from '#lib/state/workers.svelte.js';
  import { workersFailureMessage } from '#lib/utils/workers-messages.js';

  import WorkerRow from './WorkerRow.svelte';

  // The "Workers" group of the sidebar (ADR-0020, a sidebar panel;
  // docs/features/workers/worker-activity.md): between the projects and the
  // conversations, with "+", which pairs a worker, and a row for each paired
  // worker. Without workers the group is not shown, and nothing shows while
  // the list is not known yet. When the list cannot be read, the group says
  // why, with "Try again".
  const shown = $derived(workerList.status === 'error' || workerList.workers.length > 0);

  onMount(() => {
    workerList.start();
  });
</script>

{#if shown}
  <Sidebar.Group class="group-data-[collapsible=icon]:hidden" data-workers-panel="">
    <Sidebar.GroupLabel>{m.workers_panel_title()}</Sidebar.GroupLabel>
    <Sidebar.GroupAction data-pair-worker="" title={m.workers_pair_label()} onclick={() => workersActions.startPair()}>
      <PlusIcon />
      <span class="sr-only">{m.workers_pair_label()}</span>
    </Sidebar.GroupAction>
    <Sidebar.GroupContent>
      {#if workerList.status === 'error'}
        <div class="flex flex-col items-start gap-2 px-2 text-xs text-muted-foreground" role="alert">
          <p class="flex items-start gap-1.5">
            <CircleAlertIcon class="mt-px size-3.5 shrink-0 text-destructive" />
            <span>
              {m.workers_list_failed_title()}
              {workerList.failure === null ? '' : workersFailureMessage(workerList.failure)}
            </span>
          </p>
          <Button size="sm" variant="outline" onclick={() => workerList.retry()}>{m.workers_try_again()}</Button>
        </div>
      {:else}
        <Sidebar.Menu>
          {#each workerList.workers as worker (worker.id)}
            <WorkerRow {worker} />
          {/each}
        </Sidebar.Menu>
      {/if}
    </Sidebar.GroupContent>
  </Sidebar.Group>
{/if}
