<script lang="ts">
  import * as Field from '@mannie/plugin-sdk/components/field';
  import * as Select from '@mannie/plugin-sdk/components/select';

  import type { DatabaseStructure, DatabaseView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { openDatabase, shownTable } from '../utils/databases-places.js';

  import DatabaseTableRows from './DatabaseTableRows.svelte';

  // The Data tab (plugins/workers/docs/databases.md, the view): the table
  // whose rows it shows, which the address keeps, and its rows. Another
  // table mounts its rows again, from its first page and without
  // conditions. Names are content.
  let { database, structure }: { database: DatabaseView; structure: DatabaseStructure } = $props();

  const tables = $derived(structure.tables.filter((table) => table.kind !== 'shadow'));
  const table = $derived(tables.find((candidate) => candidate.name === shownTable()) ?? tables.at(0));

  function chooseTable(name: string): void {
    void openDatabase(database.key, 'data', name, { replace: true });
  }
</script>

{#if table === undefined}
  <p class="text-sm text-muted-foreground">{m.workers_databases_tables_empty()}</p>
{:else}
  <div class="flex flex-col gap-4">
    <Field.Field class="w-64">
      <Field.Label for="database-data-table">{m.workers_databases_table_select()}</Field.Label>
      <Select.Root type="single" value={table.name} onValueChange={chooseTable}>
        <Select.Trigger id="database-data-table" class="w-full">
          <span class="truncate" data-content dir="auto">{table.name}</span>
        </Select.Trigger>
        <Select.Content>
          {#each tables as candidate (candidate.name)}
            <Select.Item label={candidate.name} value={candidate.name}>
              <span class="truncate" data-content dir="auto">{candidate.name}</span>
            </Select.Item>
          {/each}
        </Select.Content>
      </Select.Root>
    </Field.Field>
    {#key table.name}
      <DatabaseTableRows {database} {table} />
    {/key}
  </div>
{/if}
