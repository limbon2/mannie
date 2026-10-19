<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import * as Item from '@mannie/plugin-sdk/components/item';
  import { Skeleton } from '@mannie/plugin-sdk/components/skeleton';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import ServerIcon from '@mannie/plugin-sdk/icons/server';
  import { onMount, tick } from 'svelte';

  import { m } from '../generated/messages.js';
  import { workersActions } from '../state/workers-actions.svelte.js';
  import { workerList } from '../state/workers.svelte.js';
  import { workersFailureMessage } from '../utils/workers-messages.js';

  import WorkerDetails from './WorkerDetails.svelte';

  // The Workers section of the settings (plugins/workers/docs/
  // worker-activity.md, the Workers settings): the paired workers with their
  // status and what they said about themselves, "Pair a worker…", and
  // without workers how to install and pair one. "Details" in the sidebar
  // opens this section with that worker's details open.
  let openFor = $state<string | null>(null);

  onMount(() => {
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
