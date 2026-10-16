<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import ServerIcon from '@lucide/svelte/icons/server';
  import { onMount, tick } from 'svelte';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Empty from '#lib/components/ui/empty/index.js';
  import * as Item from '#lib/components/ui/item/index.js';
  import { Skeleton } from '#lib/components/ui/skeleton/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { workersActions } from '#lib/state/workers-actions.svelte.js';
  import { workerList } from '#lib/state/workers.svelte.js';
  import { workersFailureMessage } from '#lib/utils/workers-messages.js';

  import WorkerDetails from './WorkerDetails.svelte';

  // The Workers section of the settings (docs/features/workers/
  // worker-activity.md, the Workers settings): the paired workers with their
  // status and what they said about themselves, "Pair a worker…", and
  // without workers how to install and pair one. "Details" in the sidebar
  // opens this section with that worker's details open.
  let openFor = $state<string | null>(null);

  onMount(() => {
    workerList.start();
    openFor = workersActions.takeDetailsFor();
    if (openFor !== null) {
      const id = openFor;
      void tick().then(() => {
        document.querySelector(`[data-worker-details="${CSS.escape(id)}"]`)?.scrollIntoView({ block: 'nearest' });
      });
    }
  });
</script>

{#if workerList.status === 'error'}
  <Alert.Root variant="destructive">
    <CircleAlertIcon />
    <Alert.Title>{m.workers_list_failed_title()}</Alert.Title>
    <Alert.Description>{workerList.failure === null ? '' : workersFailureMessage(workerList.failure)}</Alert.Description
    >
    <Alert.Action>
      <Button size="sm" variant="outline" onclick={() => workerList.retry()}>{m.workers_try_again()}</Button>
    </Alert.Action>
  </Alert.Root>
{:else if workerList.status === 'loading'}
  <div class="flex flex-col gap-3" aria-busy="true">
    <span class="sr-only" role="status">{m.workers_loading()}</span>
    <Skeleton class="h-24 w-full" />
    <Skeleton class="h-24 w-full" />
  </div>
{:else if workerList.workers.length === 0}
  <Empty.Root class="border">
    <Empty.Header>
      <Empty.Media variant="icon">
        <ServerIcon />
      </Empty.Media>
      <Empty.Title>{m.workers_settings_empty_title()}</Empty.Title>
      <Empty.Description>{m.workers_settings_empty_description()}</Empty.Description>
    </Empty.Header>
    <Empty.Content>
      <Button data-pair-worker-settings="" onclick={() => workersActions.startPair()}>{m.workers_pair()}</Button>
    </Empty.Content>
  </Empty.Root>
{:else}
  <div class="flex flex-col gap-4">
    <div>
      <Button data-pair-worker-settings="" variant="outline" onclick={() => workersActions.startPair()}>
        {m.workers_pair()}
      </Button>
    </div>
    <Item.Group>
      {#each workerList.workers as worker (worker.id)}
        <WorkerDetails open={openFor === worker.id} {worker} />
      {/each}
    </Item.Group>
  </div>
{/if}
