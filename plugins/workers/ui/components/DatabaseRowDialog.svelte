<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import * as Select from '@mannie/plugin-sdk/components/select';
  import { Textarea } from '@mannie/plugin-sdk/components/textarea';
  import { formatBytes } from '@mannie/plugin-sdk/format';
  import BanIcon from '@mannie/plugin-sdk/icons/ban';
  import Undo2Icon from '@mannie/plugin-sdk/icons/undo-2';

  import type { TableInfo } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import type { RowEditRequest } from '../state/database-rows.svelte.js';
  import { databasesFailureMessage } from '../utils/databases-messages.js';
  import { blobBytes, fieldProblem, type TypedKind } from '../utils/databases-values.js';

  // The Data tab's row dialog (plugins/workers/docs/databases.md, the view):
  // a field for each column, which starts with the row's value, or empty
  // with the column's default for a new row. A column's declared type gives
  // the kind of value it writes; only a column without one, or with BLOB
  // affinity, offers the kind. Each field can be set to NULL where the
  // column allows it, and a changed field can be undone to its start. An
  // edit writes only the columns that the owner changed, and the core checks
  // that the row still has the values that the page showed; an insertion
  // leaves the columns that keep their default out. Generated columns show
  // without a field, and binary data can only become NULL. Names and values
  // are content.
  let {
    open = $bindable(false),
    request,
    table,
    onsave,
  }: {
    onsave: (request: RowEditRequest) => void;
    open?: boolean;
    request: RowEditRequest;
    table: TableInfo;
  } = $props();

  /** The kinds that a column without a type offers. */
  const KINDS: readonly TypedKind[] = ['text', 'integer', 'real'];

  const generated = $derived(new Set(table.columns.filter((column) => column.hidden).map((column) => column.name)));
  const problems = $derived(request.fields.map((field) => fieldProblem(field)));
  const hasProblem = $derived(problems.some((problem) => problem !== null));

  function kindLabel(kind: TypedKind): string {
    switch (kind) {
      case 'integer': {
        return m.workers_databases_row_kind_integer();
      }
      case 'real': {
        return m.workers_databases_row_kind_real();
      }
      case 'text': {
        return m.workers_databases_row_kind_text();
      }
    }
  }

  function setKind(index: number, value: string): void {
    const kind = KINDS.find((known) => known === value);
    if (kind !== undefined) {
      request.setKind(index, kind);
    }
  }

  /** Whether a column takes NULL; a column that the table does not describe is taken to. */
  function allowsNull(name: string): boolean {
    return table.columns.find((column) => column.name === name)?.notNull !== true;
  }

  /** What an empty field shows: NULL, or for a new row the column's default. */
  function placeholderOf(index: number, name: string): string {
    if (request.isNull(index)) {
      return m.workers_databases_row_kind_null();
    }
    if (request.fields[index].kind !== 'default') {
      return '';
    }
    const defaultSql = table.columns.find((column) => column.name === name)?.default ?? null;
    return defaultSql === null
      ? m.workers_databases_row_kind_default()
      : m.workers_databases_row_default_value({ value: defaultSql });
  }

  /** Whether a field offers the kind of value: its column has no type, and the row has no binary data in it. */
  function offersKind(index: number): boolean {
    if (request.typed[index]) {
      return false;
    }
    return blobSize(index) === null;
  }

  /** The size of the binary data that the row has in a column, or `null` when it has none. */
  function blobSize(index: number): string | null {
    const value = request.row?.values[index];
    return value?.type === 'blob' ? formatBytes(blobBytes(value.base64)) : null;
  }

  function save(event: SubmitEvent): void {
    event.preventDefault();
    if (!request.saving && !hasProblem) {
      onsave(request);
    }
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="sm:max-w-2xl" showCloseButton={false}>
    <form class="flex flex-col gap-4" data-database-row-dialog="" onsubmit={save}>
      <Dialog.Header>
        <Dialog.Title class="wrap-anywhere">
          {request.number === null
            ? m.workers_databases_row_insert_title({ table: table.name })
            : m.workers_databases_row_edit_title({ number: request.number })}
        </Dialog.Title>
      </Dialog.Header>
      <Field.Group>
        {#each request.fields as field, index (index)}
          {const column = $derived(request.columns[index])}
          {const kind = $derived(request.kinds[index])}
          {const blob = $derived(field.kind === 'unchanged' ? blobSize(index) : null)}
          <Field.Field data-invalid={problems[index] !== null ? true : undefined}>
            <Field.Label class="font-mono" for="database-row-{index}">
              <span class="wrap-anywhere" data-content dir="auto">{column.name}</span>
              {#if column.declaredType !== null}
                <span class="text-muted-foreground" data-content>{column.declaredType}</span>
              {/if}
            </Field.Label>
            {#if generated.has(column.name)}
              <p class="text-sm text-muted-foreground">{m.workers_databases_row_generated()}</p>
            {:else}
              <div class="flex items-start gap-2">
                {#if offersKind(index)}
                  <Select.Root type="single" value={kind} onValueChange={(value: string) => setKind(index, value)}>
                    <Select.Trigger
                      class="w-36 shrink-0"
                      aria-label={m.workers_databases_row_kind({ column: column.name })}
                    >
                      {kindLabel(kind)}
                    </Select.Trigger>
                    <Select.Content>
                      {#each KINDS as known (known)}
                        <Select.Item label={kindLabel(known)} value={known}>{kindLabel(known)}</Select.Item>
                      {/each}
                    </Select.Content>
                  </Select.Root>
                {/if}
                {#if blob !== null}
                  <p
                    id="database-row-{index}"
                    class="flex h-8 min-w-0 flex-1 items-center text-sm text-muted-foreground"
                  >
                    {m.workers_databases_row_blob({ size: blob })}
                  </p>
                {:else if kind === 'text'}
                  <Textarea
                    id="database-row-{index}"
                    class="min-h-8 flex-1 font-mono"
                    dir="auto"
                    placeholder={placeholderOf(index, column.name)}
                    rows={1}
                    spellcheck="false"
                    value={field.text}
                    oninput={(event: Event) => request.edit(index, (event.currentTarget as HTMLTextAreaElement).value)}
                  />
                {:else}
                  <Input
                    id="database-row-{index}"
                    class="flex-1 font-mono"
                    aria-invalid={problems[index] !== null}
                    inputmode="decimal"
                    placeholder={placeholderOf(index, column.name)}
                    spellcheck="false"
                    value={field.text}
                    oninput={(event: Event) => request.edit(index, (event.currentTarget as HTMLInputElement).value)}
                  />
                {/if}
                {#if allowsNull(column.name) && !request.isNull(index)}
                  <Button
                    aria-label={m.workers_databases_row_set_null({ column: column.name })}
                    size="icon"
                    title={m.workers_databases_row_set_null({ column: column.name })}
                    variant="ghost"
                    onclick={() => request.setNull(index)}
                  >
                    <BanIcon />
                  </Button>
                {/if}
                {#if request.changed(index)}
                  <Button
                    aria-label={m.workers_databases_row_undo({ column: column.name })}
                    data-database-row-undo=""
                    size="icon"
                    title={m.workers_databases_row_undo({ column: column.name })}
                    variant="ghost"
                    onclick={() => request.undo(index)}
                  >
                    <Undo2Icon />
                  </Button>
                {/if}
              </div>
              {#if problems[index] !== null}
                <Field.Error>{problems[index]}</Field.Error>
              {/if}
            {/if}
          </Field.Field>
        {/each}
      </Field.Group>
      {#if request.failure !== null}
        <p class="text-sm text-destructive" role="alert">{databasesFailureMessage(request.failure)}</p>
      {/if}
      <Dialog.Footer>
        <Button type="button" variant="outline" onclick={() => (open = false)}>{m.workers_databases_cancel()}</Button>
        <Button data-database-row-save="" disabled={request.saving || hasProblem} type="submit">
          {m.workers_databases_row_save()}
        </Button>
      </Dialog.Footer>
    </form>
  </Dialog.Content>
</Dialog.Root>
