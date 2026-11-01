<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Table from '@mannie/plugin-sdk/components/table';
  import { call } from '@mannie/plugin-sdk/ipc';
  import { SvelteMap } from 'svelte/reactivity';

  import { commands, type DatabaseStructure, type DatabaseView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import type { DatabasesFailure } from '../types/databases.js';
  import { databasesFailureMessage, tableKindLabel } from '../utils/databases-messages.js';
  import { openDatabase } from '../utils/databases-places.js';

  // The Tables tab (plugins/workers/docs/databases.md, the view): the
  // database's tables and views, each of which opens on the Data tab, with
  // "Count rows" on request, since counting a large table takes time; the
  // core stops a count after 10 seconds. Shadow tables, which virtual tables
  // keep their data in, are left out. Names are content.
  type Count =
    { failure: DatabasesFailure; status: 'failed' } | { rows: string; status: 'counted' } | { status: 'counting' };

  let { database, structure }: { database: DatabaseView; structure: DatabaseStructure } = $props();

  const tables = $derived(structure.tables.filter((table) => table.kind !== 'shadow'));
  const counts = new SvelteMap<string, Count>();
  const numbers = new Intl.NumberFormat();

  async function count(table: string): Promise<void> {
    counts.set(table, { status: 'counting' });
    const outcome = await call(() => commands.countDatabaseRows({ key: database.key, table }));
    counts.set(
      table,
      outcome.ok ? { rows: outcome.data, status: 'counted' } : { failure: outcome.failure, status: 'failed' },
    );
  }

  /** A count of rows in the owner's locale; the core sends it as decimal text, which may pass JavaScript's numbers. */
  function rowsText(rows: string): string {
    return numbers.format(BigInt(rows));
  }
</script>

{#if tables.length === 0}
  <p class="text-sm text-muted-foreground" data-database-tables-empty="">{m.workers_databases_tables_empty()}</p>
{:else}
  <Table.Root data-database-tables="">
    <Table.Header>
      <Table.Row>
        <Table.Head class="text-start">{m.workers_databases_tables_name()}</Table.Head>
        <Table.Head class="text-start">{m.workers_databases_tables_kind()}</Table.Head>
        <Table.Head class="text-end">{m.workers_databases_tables_rows()}</Table.Head>
      </Table.Row>
    </Table.Header>
    <Table.Body>
      {#each tables as table (table.name)}
        {const shown = $derived(counts.get(table.name))}
        <Table.Row data-database-table={table.name}>
          <Table.Cell>
            <Button
              class="h-auto p-0"
              aria-label={m.workers_databases_tables_open({ table: table.name })}
              variant="link"
              onclick={() => void openDatabase(database.key, 'data', table.name)}
            >
              <span class="wrap-anywhere" data-content dir="auto">{table.name}</span>
            </Button>
          </Table.Cell>
          <Table.Cell>{tableKindLabel(table.kind)}</Table.Cell>
          <Table.Cell class="text-end" aria-live="polite">
            {#if shown === undefined}
              <Button size="sm" variant="outline" onclick={() => void count(table.name)}>
                {m.workers_databases_tables_count()}
              </Button>
            {:else if shown.status === 'counting'}
              <span class="text-muted-foreground">{m.workers_databases_tables_counting()}</span>
            {:else if shown.status === 'counted'}
              <span class="tabular-nums" data-database-table-rows="">{rowsText(shown.rows)}</span>
            {:else}
              <span class="text-destructive" title={databasesFailureMessage(shown.failure)}>
                {m.workers_databases_tables_count_failed()}
              </span>
              <span class="sr-only">{databasesFailureMessage(shown.failure)}</span>
            {/if}
          </Table.Cell>
        </Table.Row>
      {/each}
    </Table.Body>
  </Table.Root>
{/if}
