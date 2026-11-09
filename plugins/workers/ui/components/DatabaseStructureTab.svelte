<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Table from '@mannie/plugin-sdk/components/table';
  import { call } from '@mannie/plugin-sdk/ipc';

  import { commands, type DatabaseStructure, type DatabaseView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import type { DatabasesFailure } from '../types/databases.js';
  import { databasesFailureMessage, tableKindLabel } from '../utils/databases-messages.js';
  import { shownTable } from '../utils/databases-places.js';

  // The Structure tab (plugins/workers/docs/databases.md, the view): the
  // table that the owner chose in the sidebar, which the address keeps, or
  // the first: its name and kind with "Count rows" on request, since
  // counting a large table takes time and the core stops a count after 10
  // seconds; its columns with their types, nullability, defaults and keys,
  // its indexes, foreign keys and triggers, and the statement that created
  // it. Names, types, defaults and statements are content.
  type Count =
    { failure: DatabasesFailure; status: 'failed' } | { rows: string; status: 'counted' } | { status: 'counting' };

  let { database, structure }: { database: DatabaseView; structure: DatabaseStructure } = $props();

  /** The count of the shown table's rows, which another table starts over. */
  let count: { count: Count; table: string } | null = $state(null);
  /** Counts the counts that the owner started, so that only the newest one lands. */
  let counts = 0;

  const tables = $derived(structure.tables.filter((table) => table.kind !== 'shadow'));
  const table = $derived(tables.find((candidate) => candidate.name === shownTable()) ?? tables.at(0));
  const indexes = $derived(structure.indexes.filter((index) => index.table === table?.name));
  const triggers = $derived(structure.triggers.filter((trigger) => trigger.table === table?.name));

  const shownCount = $derived(countOf(table?.name));
  const numbers = new Intl.NumberFormat();

  /** The count of a table's rows, while it is the latest that the owner started. */
  function countOf(name: string | undefined): Count | null {
    if (count === null) {
      return null;
    }
    return count.table === name ? count.count : null;
  }

  async function countRows(name: string): Promise<void> {
    counts += 1;
    const started = counts;
    count = { count: { status: 'counting' }, table: name };
    const outcome = await call(() => commands.countDatabaseRows({ key: database.key, table: name }));
    if (started !== counts) {
      return;
    }
    count = {
      count: outcome.ok ? { rows: outcome.data, status: 'counted' } : { failure: outcome.failure, status: 'failed' },
      table: name,
    };
  }

  /** A count of rows in the owner's locale; the core sends it as decimal text, which may pass JavaScript's numbers. */
  function rowsText(rows: string): string {
    return numbers.format(BigInt(rows));
  }
</script>

{#if table === undefined}
  <p class="text-sm text-muted-foreground">{m.workers_databases_tables_empty()}</p>
{:else}
  <div class="flex flex-col gap-6" data-database-structure="">
    <div class="flex flex-wrap items-center gap-x-3 gap-y-1">
      <h2 class="font-mono text-base font-semibold wrap-anywhere" data-content dir="auto">{table.name}</h2>
      <span class="text-sm text-muted-foreground">{tableKindLabel(table.kind)}</span>
      <span class="ms-auto text-sm" aria-live="polite">
        {#if shownCount === null}
          <Button size="sm" variant="outline" onclick={() => void countRows(table.name)}>
            {m.workers_databases_tables_count()}
          </Button>
        {:else if shownCount.status === 'counting'}
          <span class="text-muted-foreground">{m.workers_databases_tables_counting()}</span>
        {:else if shownCount.status === 'counted'}
          <span class="tabular-nums" data-database-table-rows="">
            {m.workers_databases_tables_rows_counted({ rows: rowsText(shownCount.rows) })}
          </span>
        {:else}
          <span class="text-destructive" role="alert">
            {m.workers_databases_tables_count_failed({ reason: databasesFailureMessage(shownCount.failure) })}
          </span>
        {/if}
      </span>
    </div>

    <section class="flex flex-col gap-2">
      <h2 class="text-sm font-semibold">{m.workers_databases_structure_columns()}</h2>
      <Table.Root>
        <Table.Header>
          <Table.Row>
            <Table.Head class="text-start">{m.workers_databases_structure_column_name()}</Table.Head>
            <Table.Head class="text-start">{m.workers_databases_structure_column_type()}</Table.Head>
            <Table.Head class="text-start">{m.workers_databases_structure_column_not_null()}</Table.Head>
            <Table.Head class="text-start">{m.workers_databases_structure_column_default()}</Table.Head>
            <Table.Head class="text-start">{m.workers_databases_structure_column_key()}</Table.Head>
          </Table.Row>
        </Table.Header>
        <Table.Body>
          {#each table.columns as column (column.name)}
            <Table.Row>
              <Table.Cell class="font-mono" data-content dir="auto">{column.name}</Table.Cell>
              <Table.Cell class="font-mono" data-content dir="auto">{column.declaredType}</Table.Cell>
              <Table.Cell>{column.notNull ? m.workers_databases_structure_yes() : ''}</Table.Cell>
              <Table.Cell class="font-mono" data-content dir="auto">{column.default ?? ''}</Table.Cell>
              <Table.Cell class="tabular-nums">{column.primaryKey > 0 ? column.primaryKey : ''}</Table.Cell>
            </Table.Row>
          {/each}
        </Table.Body>
      </Table.Root>
    </section>

    <section class="flex flex-col gap-2">
      <h2 class="text-sm font-semibold">{m.workers_databases_structure_indexes()}</h2>
      {#if indexes.length === 0}
        <p class="text-sm text-muted-foreground">{m.workers_databases_structure_none()}</p>
      {:else}
        <ul class="flex flex-col gap-1 text-sm">
          {#each indexes as index (index.name)}
            <li class="flex flex-wrap items-baseline gap-2">
              <span class="font-mono wrap-anywhere" data-content dir="auto">{index.name}</span>
              <span class="font-mono wrap-anywhere text-muted-foreground" data-content dir="auto">
                ({index.columns.map((column) => column ?? m.workers_databases_structure_expression()).join(', ')})
              </span>
              {#if index.unique}<span class="text-xs">{m.workers_databases_structure_index_unique()}</span>{/if}
              {#if index.partial}<span class="text-xs">{m.workers_databases_structure_index_partial()}</span>{/if}
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section class="flex flex-col gap-2">
      <h2 class="text-sm font-semibold">{m.workers_databases_structure_foreign_keys()}</h2>
      {#if table.foreignKeys.length === 0}
        <p class="text-sm text-muted-foreground">{m.workers_databases_structure_none()}</p>
      {:else}
        <ul class="flex flex-col gap-1 text-sm">
          {#each table.foreignKeys as key, index (index)}
            <li class="font-mono wrap-anywhere" data-content dir="auto">
              {m.workers_databases_structure_references({
                columns: key.columns.join(', '),
                onDelete: key.onDelete,
                parent: key.parent,
                parentColumns: key.parentColumns
                  .map((column) => column ?? m.workers_databases_structure_primary_key())
                  .join(', '),
              })}
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section class="flex flex-col gap-2">
      <h2 class="text-sm font-semibold">{m.workers_databases_structure_triggers()}</h2>
      {#if triggers.length === 0}
        <p class="text-sm text-muted-foreground">{m.workers_databases_structure_none()}</p>
      {:else}
        <ul class="flex flex-col gap-1 text-sm">
          {#each triggers as trigger (trigger.name)}
            <li class="font-mono wrap-anywhere" data-content dir="auto">{trigger.name}</li>
          {/each}
        </ul>
      {/if}
    </section>

    {#if table.sql !== null}
      <section class="flex flex-col gap-2">
        <h2 class="text-sm font-semibold">{m.workers_databases_structure_sql()}</h2>
        <pre
          class="overflow-auto rounded-md bg-muted px-3 py-2 font-mono text-xs whitespace-pre-wrap"
          data-content
          dir="ltr">{table.sql}</pre>
      </section>
    {/if}
  </div>
{/if}
