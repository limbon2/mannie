<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Sidebar from '@mannie/plugin-sdk/components/sidebar';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';

  import { m } from '../generated/messages.js';
  import { databaseActions } from '../state/database-actions.svelte.js';
  import { databaseList } from '../state/databases.svelte.js';
  import { databasesFailureMessage } from '../utils/databases-messages.js';

  import DatabaseRow from './DatabaseRow.svelte';

  // The sidebar of the Databases view (plugins/workers/docs/databases.md,
  // the view): the databases on this computer, with "+", which adds one,
  // and a row for each with its state. The workers' databases join as groups
  // of their own with ADR-0057. While the list is not known it says so, and
  // when it cannot be read it says why, with "Retry".
</script>

<Sidebar.Group data-databases-panel="">
  <Sidebar.GroupLabel>{m.workers_databases_machine_this_computer()}</Sidebar.GroupLabel>
  <Sidebar.GroupAction
    data-add-database=""
    title={m.workers_databases_add()}
    onclick={() => databaseActions.startAdd()}
  >
    <PlusIcon />
    <span class="sr-only">{m.workers_databases_add()}</span>
  </Sidebar.GroupAction>
  <Sidebar.GroupContent>
    {#if databaseList.status === 'loading'}
      <p class="px-2 text-xs text-muted-foreground" data-databases-loading="">{m.workers_databases_loading()}</p>
    {:else if databaseList.status === 'error'}
      <div class="flex flex-col items-start gap-2 px-2 text-xs text-muted-foreground" role="alert">
        <p class="flex items-start gap-1.5">
          <CircleAlertIcon class="mt-px size-3.5 shrink-0 text-destructive" />
          <span>
            {m.workers_databases_list_failed()}
            {databaseList.failure === null ? '' : databasesFailureMessage(databaseList.failure)}
          </span>
        </p>
        <Button size="sm" variant="outline" onclick={() => databaseList.retry()}>{m.workers_databases_retry()}</Button>
      </div>
    {:else if databaseList.databases.length === 0}
      <p class="px-2 text-xs text-muted-foreground" data-databases-empty="">{m.workers_databases_empty()}</p>
    {:else}
      <Sidebar.Menu>
        {#each databaseList.databases as database (database.key)}
          <DatabaseRow {database} />
        {/each}
      </Sidebar.Menu>
    {/if}
  </Sidebar.GroupContent>
</Sidebar.Group>
