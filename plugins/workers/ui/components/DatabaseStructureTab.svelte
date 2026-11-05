<script lang="ts">
  import * as Field from '@mannie/plugin-sdk/components/field';
  import * as Select from '@mannie/plugin-sdk/components/select';
  import * as Table from '@mannie/plugin-sdk/components/table';

  import type { DatabaseStructure, DatabaseView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { tableKindLabel } from '../utils/databases-messages.js';
  import { openDatabase, shownTable } from '../utils/databases-places.js';

  // The Structure tab (plugins/workers/docs/databases.md, the view): one
  // table's columns with their types, nullability, defaults and keys, its
  // indexes, foreign keys and triggers, and the statement that created it.
  // The address keeps the chosen table. Names, types, defaults and
  // statements are content.
  let { database, structure }: { database: DatabaseView; structure: DatabaseStructure } = $props();

  const tables = $derived(structure.tables.filter((table) => table.kind !== 'shadow'));
  const table = $derived(tables.find((candidate) => candidate.name === shownTable()) ?? tables.at(0));
  const indexes = $derived(structure.indexes.filter((index) => index.table === table?.name));
  const triggers = $derived(structure.triggers.filter((trigger) => trigger.table === table?.name));

  function choose(name: string): void {
    void openDatabase(database.key, 'structure', name, { replace: true });
  }
</script>

{#if table === undefined}
  <p class="text-sm text-muted-foreground">{m.workers_databases_tables_empty()}</p>
{:else}
  <div class="flex flex-col gap-6" data-database-structure="">
    <Field.Field class="max-w-sm">
      <Field.Label for="database-structure-table">{m.workers_databases_table_select()}</Field.Label>
      <Select.Root type="single" value={table.name} onValueChange={choose}>
        <Select.Trigger id="database-structure-table" class="w-full">
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
      <Field.Description>{tableKindLabel(table.kind)}</Field.Description>
    </Field.Field>

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
