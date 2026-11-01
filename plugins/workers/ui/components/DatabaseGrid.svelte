<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { Checkbox } from '@mannie/plugin-sdk/components/checkbox';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Table from '@mannie/plugin-sdk/components/table';
  import PencilIcon from '@mannie/plugin-sdk/icons/pencil';
  import type { SvelteSet } from 'svelte/reactivity';

  import type { ResultColumn, SqlValue } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { cellText, fullText, isWorded } from '../utils/databases-values.js';

  // Rows of a database in a table (plugins/workers/docs/databases.md, the
  // view): a column for each of the result's columns, with its values cut
  // to one line, NULL and binary data worded and muted. A value opens whole
  // in a dialog. The Data tab passes the rows that the owner selected and an
  // edit action; the SQL tab shows rows only. Names and values are content.
  let {
    columns,
    rows,
    selected = null,
    onedit = null,
  }: {
    columns: readonly ResultColumn[];
    onedit?: ((index: number) => void) | null;
    rows: readonly (readonly SqlValue[])[];
    selected?: SvelteSet<number> | null;
  } = $props();

  /** The value that the dialog shows whole, with its column's name. */
  let shown: { column: string; value: SqlValue } | null = $state(null);
  let shownOpen = $state(false);

  const everySelected = $derived(selected !== null && rows.length > 0 && selected.size === rows.length);
  const someSelected = $derived(selected !== null && selected.size > 0 && !everySelected);

  function show(column: string, value: SqlValue): void {
    shown = { column, value };
    shownOpen = true;
  }

  function edit(index: number): void {
    onedit?.(index);
  }

  function selectPage(checked: boolean): void {
    const chosen = selected;
    if (chosen === null) {
      return;
    }
    chosen.clear();
    if (checked) {
      rows.forEach((_row, index) => chosen.add(index));
    }
  }

  function selectRow(index: number, checked: boolean): void {
    const chosen = selected;
    if (chosen === null) {
      return;
    }
    if (checked) {
      chosen.add(index);
    } else {
      chosen.delete(index);
    }
  }
</script>

<Table.Root data-database-grid="">
  <Table.Header>
    <Table.Row>
      {#if selected !== null}
        <Table.Head class="w-8">
          <Checkbox
            aria-label={m.workers_databases_data_select_page()}
            checked={everySelected}
            indeterminate={someSelected}
            onCheckedChange={(checked: boolean) => selectPage(checked)}
          />
        </Table.Head>
      {/if}
      {#if onedit !== null}
        <Table.Head class="w-8"></Table.Head>
      {/if}
      {#each columns as column, index (index)}
        <Table.Head class="text-start font-mono" data-content dir="auto">{column.name}</Table.Head>
      {/each}
    </Table.Row>
  </Table.Header>
  <Table.Body>
    {#each rows as row, rowIndex (rowIndex)}
      <Table.Row data-state={selected?.has(rowIndex) === true ? 'selected' : undefined}>
        {#if selected !== null}
          <Table.Cell>
            <Checkbox
              aria-label={m.workers_databases_data_select_row({ number: rowIndex + 1 })}
              checked={selected.has(rowIndex)}
              onCheckedChange={(checked: boolean) => selectRow(rowIndex, checked)}
            />
          </Table.Cell>
        {/if}
        {#if onedit !== null}
          <Table.Cell>
            <Button size="icon-xs" variant="ghost" onclick={() => edit(rowIndex)}>
              <PencilIcon />
              <span class="sr-only">{m.workers_databases_data_edit({ number: rowIndex + 1 })}</span>
            </Button>
          </Table.Cell>
        {/if}
        {#each row as value, valueIndex (valueIndex)}
          {#if isWorded(value)}
            <Table.Cell class="max-w-80 truncate text-muted-foreground italic">{cellText(value)}</Table.Cell>
          {:else}
            <Table.Cell class="max-w-80 p-0">
              <Button
                class="h-auto w-full max-w-80 justify-start truncate p-2 font-mono font-normal"
                data-content
                dir="auto"
                variant="ghost"
                onclick={() => show(columns[valueIndex].name, value)}
              >
                {cellText(value)}
              </Button>
            </Table.Cell>
          {/if}
        {/each}
      </Table.Row>
    {/each}
  </Table.Body>
</Table.Root>

<Dialog.Root bind:open={shownOpen}>
  <Dialog.Content class="sm:max-w-2xl" data-database-value-dialog="" showCloseButton={false}>
    {#if shown !== null}
      <Dialog.Header>
        <Dialog.Title class="font-mono wrap-anywhere" data-content dir="auto">{shown.column}</Dialog.Title>
      </Dialog.Header>
      <pre
        class="max-h-[60vh] overflow-auto rounded-md bg-muted px-3 py-2 font-mono text-xs break-all whitespace-pre-wrap"
        data-content
        dir="auto">{fullText(shown.value)}</pre>
      <Dialog.Footer>
        <Button variant="outline" onclick={() => (shownOpen = false)}>{m.workers_databases_close()}</Button>
      </Dialog.Footer>
    {/if}
  </Dialog.Content>
</Dialog.Root>
