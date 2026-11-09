<script lang="ts">
  import * as AlertDialog from '@mannie/plugin-sdk/components/alert-dialog';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { Checkbox } from '@mannie/plugin-sdk/components/checkbox';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import * as Select from '@mannie/plugin-sdk/components/select';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import ChevronLeftIcon from '@mannie/plugin-sdk/icons/chevron-left';
  import ChevronRightIcon from '@mannie/plugin-sdk/icons/chevron-right';
  import CopyIcon from '@mannie/plugin-sdk/icons/copy';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';
  import TrashIcon from '@mannie/plugin-sdk/icons/trash';
  import XIcon from '@mannie/plugin-sdk/icons/x';
  import { call } from '@mannie/plugin-sdk/ipc';
  import { onMount } from 'svelte';
  import { SvelteSet } from 'svelte/reactivity';

  import {
    type Change,
    commands,
    type Cursor,
    type DatabaseView,
    type Filter,
    type FilterOperator,
    type Page,
    type TableInfo,
  } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { RowEditRequest } from '../state/database-rows.svelte.js';
  import type { DatabasesFailure } from '../types/databases.js';
  import { deleteChange, insertChange, ownerChange, updateChange } from '../utils/databases-changes.js';
  import { databasesFailureMessage, operatorLabel } from '../utils/databases-messages.js';
  import { pageCsv } from '../utils/databases-values.js';

  import DatabaseGrid from './DatabaseGrid.svelte';
  import DatabaseRowDialog from './DatabaseRowDialog.svelte';

  // One table's rows on the Data tab (plugins/workers/docs/databases.md,
  // the view): pages of 100, in the order of its key or of a column, with
  // conditions on columns that the core turns into a parameterized
  // statement. A table with a key edits, inserts and deletes rows; each
  // change names its rows by their keys and the values that the page showed.
  // "Copy page as CSV" copies the page. The order, the conditions, the pages
  // and the actions on rows share one compact row above the rows, the
  // actions as icons whose names show on hover, and conditions show below it
  // once the owner adds one. The tab mounts it again for another table,
  // which starts from its first page without conditions. Names and values
  // are content.
  /** A condition as the owner writes it. */
  interface Condition {
    column: string;
    id: number;
    operator: FilterOperator;
    value: string;
  }

  /** The rows of a page. */
  const PAGE_ROWS = 100;

  /** The value of the order's choice that sorts by the key. */
  const KEY_ORDER = '';

  const OPERATORS: readonly FilterOperator[] = [
    'equal',
    'not_equal',
    'less',
    'less_or_equal',
    'greater',
    'greater_or_equal',
    'like',
    'is_null',
    'is_not_null',
  ];

  /** The comparisons that need no value. */
  const WITHOUT_VALUE: ReadonlySet<FilterOperator> = new Set(['is_null', 'is_not_null']);

  let { database, table }: { database: DatabaseView; table: TableInfo } = $props();

  let orderColumn = $state(KEY_ORDER);
  let descending = $state(false);
  let conditions: Condition[] = $state([]);
  let applied: Filter[] = $state([]);
  let nextCondition = 0;
  /** Where each page shown so far starts; the last is the page on show. */
  let starts: (Cursor | null)[] = $state([null]);
  let page = $state<Page | null>(null);
  let failure: DatabasesFailure | null = $state(null);
  let loading = $state(false);
  let notice: string | null = $state(null);
  /** Counts reads of pages, so that a slower earlier one never covers a newer one. */
  let reads = 0;
  const selected = new SvelteSet<number>();
  let editing: RowEditRequest | null = $state(null);
  let editOpen = $state(false);
  let deleteOpen = $state(false);
  let deleting = $state(false);
  let deleteFailure: DatabasesFailure | null = $state(null);

  const columnNames = $derived(table.columns.map((column) => column.name));
  const hasKey = $derived(page !== null && page.key.length > 0);
  const editable = $derived(table.kind === 'table' && !database.readOnly && hasKey);
  const insertable = $derived(table.kind === 'table' && !database.readOnly);

  async function load(after: Cursor | null): Promise<void> {
    reads += 1;
    const read = reads;
    loading = true;
    failure = null;
    const order = orderColumn === KEY_ORDER ? null : { column: orderColumn, descending };
    const request = { after, filters: applied, order, rows: PAGE_ROWS, table: table.name };
    const outcome = await call(() => commands.databasePage({ key: database.key, request }));
    if (read !== reads) {
      return;
    }
    loading = false;
    selected.clear();
    if (outcome.ok) {
      page = outcome.data;
    } else {
      failure = outcome.failure;
    }
  }

  function showFirst(): void {
    starts = [null];
    void load(null);
  }

  function showNext(): void {
    const next = page?.next ?? null;
    if (next !== null) {
      starts = [...starts, next];
      void load(next);
    }
  }

  function showPrevious(): void {
    if (starts.length > 1) {
      starts = starts.slice(0, -1);
      void load(starts.at(-1) ?? null);
    }
  }

  function filterOf(condition: Condition): Filter {
    const withoutValue = WITHOUT_VALUE.has(condition.operator);
    return {
      column: condition.column,
      operator: condition.operator,
      // A value compares as text, which SQLite converts to the column's
      // affinity, so a number matches a numeric column.
      value: withoutValue ? { type: 'null' } : { type: 'text', value: condition.value },
    };
  }

  function apply(event: SubmitEvent): void {
    event.preventDefault();
    applied = conditions.map(filterOf);
    showFirst();
  }

  function addCondition(): void {
    const column = columnNames.at(0);
    if (column !== undefined) {
      nextCondition += 1;
      conditions = [...conditions, { column, id: nextCondition, operator: 'equal', value: '' }];
    }
  }

  function chooseOrder(value: string): void {
    orderColumn = value;
    showFirst();
  }

  /** Runs one of the owner's changes, and shows the page again once it ran. */
  async function run(change: Change): Promise<DatabasesFailure | null> {
    const outcome = await call(() => commands.runDatabaseChange({ change: ownerChange(change), key: database.key }));
    if (!outcome.ok) {
      return outcome.failure;
    }
    void load(starts.at(-1) ?? null);
    return null;
  }

  function startEdit(index: number): void {
    const shown = page;
    if (shown === null) {
      return;
    }
    editing = new RowEditRequest(shown.columns, shown.rows[index], index + 1);
    editOpen = true;
  }

  function startInsert(): void {
    if (page !== null) {
      editing = new RowEditRequest(page.columns, null, null);
      editOpen = true;
    }
  }

  async function saveRow(request: RowEditRequest): Promise<void> {
    const change =
      request.row === null
        ? insertChange(table.name, request.columns, request.fields)
        : updateChange(table.name, request.columns, request.row, request.fields);
    if (change === null) {
      editOpen = false;
      return;
    }
    request.saving = true;
    request.failure = null;
    const failed = await run(change);
    request.saving = false;
    if (failed !== null) {
      request.failure = failed;
      return;
    }
    if (editing === request) {
      editOpen = false;
    }
    notice = m.workers_databases_row_saved();
  }

  async function deleteSelected(): Promise<void> {
    const shown = page;
    if (shown === null) {
      return;
    }
    const rows = [...selected].map((index) => shown.rows[index]);
    deleting = true;
    deleteFailure = null;
    const failed = await run(deleteChange(table.name, shown.columns, rows));
    deleting = false;
    if (failed !== null) {
      deleteFailure = failed;
      return;
    }
    deleteOpen = false;
    notice = m.workers_databases_data_deleted({ count: rows.length });
  }

  async function copyPage(): Promise<void> {
    if (page === null) {
      return;
    }
    const csv = pageCsv(
      page.columns,
      page.rows.map((row) => row.values),
    );
    try {
      await navigator.clipboard.writeText(csv);
      notice = m.workers_databases_data_copied();
    } catch {
      notice = m.workers_databases_data_copy_failed();
    }
  }

  onMount(() => {
    showFirst();
  });
</script>

<div class="flex flex-col gap-3" data-database-data="">
  <form class="flex flex-col gap-2" onsubmit={apply}>
    <div class="flex flex-wrap items-center gap-x-4 gap-y-2">
      <Field.Field class="w-auto" orientation="horizontal">
        <Field.Label class="whitespace-nowrap" for="database-data-order">{m.workers_databases_data_sort()}</Field.Label>
        <Select.Root type="single" value={orderColumn} onValueChange={chooseOrder}>
          <Select.Trigger id="database-data-order" class="w-44" size="sm">
            {#if orderColumn === KEY_ORDER}
              {m.workers_databases_data_sort_key()}
            {:else}
              <span class="truncate font-mono" data-content dir="auto">{orderColumn}</span>
            {/if}
          </Select.Trigger>
          <Select.Content>
            <Select.Item label={m.workers_databases_data_sort_key()} value={KEY_ORDER}>
              {m.workers_databases_data_sort_key()}
            </Select.Item>
            {#each columnNames as column (column)}
              <Select.Item label={column} value={column}>
                <span class="truncate font-mono" data-content dir="auto">{column}</span>
              </Select.Item>
            {/each}
          </Select.Content>
        </Select.Root>
      </Field.Field>
      <Field.Field class="w-auto" orientation="horizontal">
        <Checkbox
          id="database-data-descending"
          disabled={orderColumn === KEY_ORDER}
          bind:checked={descending}
          onCheckedChange={() => showFirst()}
        />
        <Field.Label class="whitespace-nowrap" for="database-data-descending">
          {m.workers_databases_data_descending()}
        </Field.Label>
      </Field.Field>
      <div class="flex items-center gap-2">
        <Button size="sm" type="button" variant="outline" onclick={addCondition}>
          <PlusIcon data-icon="inline-start" />
          {m.workers_databases_data_condition_add()}
        </Button>
        <Button data-database-apply="" size="sm" type="submit">{m.workers_databases_data_apply()}</Button>
      </div>
      <div class="ms-auto flex flex-wrap items-center gap-1" role="toolbar">
        {#if loading}
          <Spinner />
          <span class="sr-only">{m.workers_databases_data_loading()}</span>
        {/if}
        <Button disabled={starts.length <= 1 || loading} size="sm" variant="outline" onclick={showPrevious}>
          <ChevronLeftIcon data-icon="inline-start" />
          {m.workers_databases_data_previous()}
        </Button>
        <Button
          data-database-next=""
          disabled={(page?.next ?? null) === null || loading}
          size="sm"
          variant="outline"
          onclick={showNext}
        >
          {m.workers_databases_data_next()}
          <ChevronRightIcon data-icon="inline-end" />
        </Button>
        {#if insertable}
          <Button
            aria-label={m.workers_databases_data_insert()}
            data-database-insert=""
            disabled={page === null}
            size="icon-sm"
            title={m.workers_databases_data_insert()}
            variant="outline"
            onclick={startInsert}
          >
            <PlusIcon />
          </Button>
        {/if}
        {#if editable}
          <Button
            aria-label={m.workers_databases_data_delete()}
            data-database-delete=""
            disabled={selected.size === 0}
            size="icon-sm"
            title={m.workers_databases_data_delete()}
            variant="outline"
            onclick={() => {
              deleteFailure = null;
              deleteOpen = true;
            }}
          >
            <TrashIcon />
          </Button>
        {/if}
        <Button
          aria-label={m.workers_databases_data_copy()}
          disabled={page === null}
          size="icon-sm"
          title={m.workers_databases_data_copy()}
          variant="outline"
          onclick={() => void copyPage()}
        >
          <CopyIcon />
        </Button>
      </div>
    </div>
    {#if conditions.length > 0}
      <Field.Set class="gap-2">
        <Field.Legend class="sr-only">{m.workers_databases_data_conditions()}</Field.Legend>
        {#each conditions as condition, index (condition.id)}
          <div class="flex flex-wrap items-center gap-2" data-database-condition="">
            <Select.Root type="single" bind:value={condition.column}>
              <Select.Trigger class="w-48" aria-label={m.workers_databases_data_condition_column()} size="sm">
                <span class="truncate font-mono" data-content dir="auto">{condition.column}</span>
              </Select.Trigger>
              <Select.Content>
                {#each columnNames as column (column)}
                  <Select.Item label={column} value={column}>
                    <span class="truncate font-mono" data-content dir="auto">{column}</span>
                  </Select.Item>
                {/each}
              </Select.Content>
            </Select.Root>
            <Select.Root
              type="single"
              value={condition.operator}
              onValueChange={(value: string) => {
                const operator = OPERATORS.find((known) => known === value);
                if (operator !== undefined) {
                  condition.operator = operator;
                }
              }}
            >
              <Select.Trigger class="w-44" aria-label={m.workers_databases_data_condition_operator()} size="sm">
                {operatorLabel(condition.operator)}
              </Select.Trigger>
              <Select.Content>
                {#each OPERATORS as operator (operator)}
                  <Select.Item label={operatorLabel(operator)} value={operator}>{operatorLabel(operator)}</Select.Item>
                {/each}
              </Select.Content>
            </Select.Root>
            {#if !WITHOUT_VALUE.has(condition.operator)}
              <Input
                class="h-7 w-56 font-mono"
                aria-label={m.workers_databases_data_condition_value()}
                dir="auto"
                spellcheck="false"
                bind:value={condition.value}
              />
            {/if}
            <Button
              aria-label={m.workers_databases_data_condition_remove()}
              size="icon-sm"
              title={m.workers_databases_data_condition_remove()}
              type="button"
              variant="ghost"
              onclick={() => (conditions = conditions.filter((_kept, kept) => kept !== index))}
            >
              <XIcon />
            </Button>
          </div>
        {/each}
      </Field.Set>
    {/if}
  </form>

  {#if notice !== null}
    <p class="text-sm text-muted-foreground" aria-live="polite" data-database-notice="">{notice}</p>
  {/if}
  {#if failure !== null}
    <p class="text-sm text-destructive" role="alert">{databasesFailureMessage(failure)}</p>
  {:else if page !== null}
    {#if !hasKey}
      <p class="text-sm text-muted-foreground">{m.workers_databases_data_view_note()}</p>
    {/if}
    {#if page.rows.length === 0}
      <p class="text-sm text-muted-foreground" data-database-no-rows="">{m.workers_databases_data_empty()}</p>
    {:else}
      <DatabaseGrid
        columns={page.columns}
        rows={page.rows.map((row) => row.values)}
        selected={editable ? selected : null}
        onedit={editable ? startEdit : null}
      />
    {/if}
  {/if}
</div>

{#if editing !== null}
  <DatabaseRowDialog
    request={editing}
    {table}
    bind:open={editOpen}
    onsave={(request: RowEditRequest) => void saveRow(request)}
  />
{/if}

<AlertDialog.Root bind:open={deleteOpen}>
  <AlertDialog.Content class="data-[size=default]:sm:max-w-md" data-database-delete-dialog="">
    <AlertDialog.Header>
      <AlertDialog.Title>{m.workers_databases_data_delete_title({ count: selected.size })}</AlertDialog.Title>
      <AlertDialog.Description>{m.workers_databases_data_delete_description()}</AlertDialog.Description>
    </AlertDialog.Header>
    {#if deleteFailure !== null}
      <p class="text-sm text-destructive" role="alert">{databasesFailureMessage(deleteFailure)}</p>
    {/if}
    <AlertDialog.Footer>
      <AlertDialog.Cancel>{m.workers_databases_cancel()}</AlertDialog.Cancel>
      <Button
        data-database-delete-confirm=""
        disabled={deleting}
        variant="destructive"
        onclick={() => void deleteSelected()}
      >
        {m.workers_databases_data_delete_confirm()}
      </Button>
    </AlertDialog.Footer>
  </AlertDialog.Content>
</AlertDialog.Root>
