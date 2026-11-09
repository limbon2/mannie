<script lang="ts">
  import type { DatabaseStructure, DatabaseView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { shownTable } from '../utils/databases-places.js';

  import DatabaseTableRows from './DatabaseTableRows.svelte';

  // The Data tab (plugins/workers/docs/databases.md, the view): the rows of
  // the table that the owner chose in the sidebar, which the address keeps,
  // or of the first table. Another table mounts its rows again, from its
  // first page and without conditions.
  let { database, structure }: { database: DatabaseView; structure: DatabaseStructure } = $props();

  const tables = $derived(structure.tables.filter((table) => table.kind !== 'shadow'));
  const table = $derived(tables.find((candidate) => candidate.name === shownTable()) ?? tables.at(0));
</script>

{#if table === undefined}
  <p class="text-sm text-muted-foreground">{m.workers_databases_tables_empty()}</p>
{:else}
  {#key table.name}
    <DatabaseTableRows {database} {table} />
  {/key}
{/if}
