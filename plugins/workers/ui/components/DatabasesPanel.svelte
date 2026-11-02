<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Sidebar from '@mannie/plugin-sdk/components/sidebar';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';

  import { m } from '../generated/messages.js';
  import { databaseActions } from '../state/database-actions.svelte.js';
  import { databaseList } from '../state/databases.svelte.js';
  import { workerList } from '../state/workers.svelte.js';
  import { databaseGroups } from '../utils/databases-groups.js';
  import { databasesFailureMessage } from '../utils/databases-messages.js';

  import DatabaseRow from './DatabaseRow.svelte';

  // The sidebar of the Databases view (plugins/workers/docs/databases.md,
  // the view): the databases on this computer, then a group for each paired
  // worker that allows databases or has some (ADR-0057), each with "+",
  // which adds one, and a row for each database with its state. A worker's
  // name is content. While the list is not known it says so, and when it
  // cannot be read it says why, with "Retry".
  const groups = $derived(databaseGroups(databaseList.databases, workerList.workers));
</script>

{#if databaseList.status === 'loading'}
  <Sidebar.Group data-databases-panel="">
    <Sidebar.GroupLabel>{m.workers_databases_machine_this_computer()}</Sidebar.GroupLabel>
    <Sidebar.GroupContent>
      <p class="px-2 text-xs text-muted-foreground" data-databases-loading="">{m.workers_databases_loading()}</p>
    </Sidebar.GroupContent>
  </Sidebar.Group>
{:else if databaseList.status === 'error'}
  <Sidebar.Group data-databases-panel="">
    <Sidebar.GroupLabel>{m.workers_databases_machine_this_computer()}</Sidebar.GroupLabel>
    <Sidebar.GroupContent>
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
    </Sidebar.GroupContent>
  </Sidebar.Group>
{:else}
  {#each groups as group (group.worker?.id ?? 'this-computer')}
    {const worker = $derived(group.worker)}
    {const addLabel = $derived(
      worker === null ? m.workers_databases_add() : m.workers_databases_add_on_worker({ worker: worker.name }),
    )}
    <Sidebar.Group data-databases-machine={worker?.id ?? 'this-computer'} data-databases-panel="">
      {#if worker === null}
        <Sidebar.GroupLabel>{m.workers_databases_machine_this_computer()}</Sidebar.GroupLabel>
      {:else}
        <Sidebar.GroupLabel>
          <span class="truncate" data-content dir="auto">{worker.name}</span>
        </Sidebar.GroupLabel>
      {/if}
      {#if group.canAdd}
        <Sidebar.GroupAction data-add-database="" title={addLabel} onclick={() => databaseActions.startAdd(worker)}>
          <PlusIcon />
          <span class="sr-only">{addLabel}</span>
        </Sidebar.GroupAction>
      {/if}
      <Sidebar.GroupContent>
        {#if group.databases.length === 0}
          <p class="px-2 text-xs text-muted-foreground" data-databases-empty="">{m.workers_databases_empty()}</p>
        {:else}
          <Sidebar.Menu>
            {#each group.databases as database (database.key)}
              <DatabaseRow {database} />
            {/each}
          </Sidebar.Menu>
        {/if}
      </Sidebar.GroupContent>
    </Sidebar.Group>
  {/each}
{/if}
