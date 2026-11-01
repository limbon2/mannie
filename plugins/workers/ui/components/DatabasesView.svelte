<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import DatabaseIcon from '@mannie/plugin-sdk/icons/database';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';
  import { onMount } from 'svelte';

  import { m } from '../generated/messages.js';
  import { databaseActions } from '../state/database-actions.svelte.js';
  import { databaseList } from '../state/databases.svelte.js';
  import { databasesFailureMessage } from '../utils/databases-messages.js';
  import { shownDatabaseKey } from '../utils/databases-places.js';

  import DatabasePage from './DatabasePage.svelte';

  // The Databases view (plugins/workers/docs/databases.md, the view): the
  // page of the database that the address names, or what to do without one.
  // Opening the view looks at each registered file again, so the states that
  // the sidebar and the page show are current.
  const key = $derived(shownDatabaseKey());
  const database = $derived(key === null ? undefined : databaseList.find(key));

  onMount(() => {
    void databaseList.refresh();
  });
</script>

<div class="flex min-h-0 flex-1 flex-col" data-databases-view="">
  {#if databaseList.status === 'loading'}
    <div class="flex flex-1 items-center justify-center gap-2 text-sm text-muted-foreground">
      <Spinner />
      {m.workers_databases_loading()}
    </div>
  {:else if databaseList.status === 'error'}
    <Empty.Root>
      <Empty.Header>
        <Empty.Media variant="icon"><CircleAlertIcon /></Empty.Media>
        <Empty.Title>{m.workers_databases_list_failed()}</Empty.Title>
        <Empty.Description>
          {databaseList.failure === null ? '' : databasesFailureMessage(databaseList.failure)}
        </Empty.Description>
      </Empty.Header>
      <Empty.Content>
        <Button variant="outline" onclick={() => databaseList.retry()}>{m.workers_databases_retry()}</Button>
      </Empty.Content>
    </Empty.Root>
  {:else if databaseList.databases.length === 0}
    <Empty.Root data-databases-none="">
      <Empty.Header>
        <Empty.Media variant="icon"><DatabaseIcon /></Empty.Media>
        <Empty.Title>{m.workers_databases_empty()}</Empty.Title>
        <Empty.Description>{m.workers_databases_empty_description()}</Empty.Description>
      </Empty.Header>
      <Empty.Content>
        <Button data-databases-add="" onclick={() => databaseActions.startAdd()}>
          <PlusIcon data-icon="inline-start" />
          {m.workers_databases_add()}
        </Button>
      </Empty.Content>
    </Empty.Root>
  {:else if key === null}
    <Empty.Root>
      <Empty.Header>
        <Empty.Media variant="icon"><DatabaseIcon /></Empty.Media>
        <Empty.Title>{m.workers_databases_none_selected()}</Empty.Title>
        <Empty.Description>{m.workers_databases_none_selected_description()}</Empty.Description>
      </Empty.Header>
    </Empty.Root>
  {:else if database === undefined}
    <Empty.Root>
      <Empty.Header>
        <Empty.Media variant="icon"><CircleAlertIcon /></Empty.Media>
        <Empty.Title>{m.workers_databases_not_found()}</Empty.Title>
      </Empty.Header>
    </Empty.Root>
  {:else}
    {#key `${database.key}:${database.state}`}
      <DatabasePage {database} />
    {/key}
  {/if}
</div>
