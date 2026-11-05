<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import * as Select from '@mannie/plugin-sdk/components/select';
  import { Textarea } from '@mannie/plugin-sdk/components/textarea';

  import type { TableInfo } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import type { RowEditRequest } from '../state/database-rows.svelte.js';
  import { databasesFailureMessage } from '../utils/databases-messages.js';
  import { fieldProblem, type ValueKind } from '../utils/databases-values.js';

  // The Data tab's row dialog (plugins/workers/docs/databases.md, the view):
  // a field for each column, with the kind of value it writes. An edit
  // writes only the columns that the owner changed, and the core checks that
  // the row still has the values that the page showed; an insertion leaves
  // the columns that keep their default out. Generated columns show without
  // a field, and binary data can only become NULL. Names and values are
  // content.
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

  /** The kinds of value that are numbers. */
  const NUMBERS: ReadonlySet<ValueKind> = new Set(['integer', 'real']);

  const generated = $derived(new Set(table.columns.filter((column) => column.hidden).map((column) => column.name)));
  const problems = $derived(request.fields.map((field) => fieldProblem(field)));
  const hasProblem = $derived(problems.some((problem) => problem !== null));

  /** The kinds that a column's field offers: an edit keeps a value unchanged, an insertion takes the default. */
  function kindsOf(index: number): readonly ValueKind[] {
    const isBlob = request.row?.values[index]?.type === 'blob';
    if (request.row === null) {
      return ['default', 'text', 'integer', 'real', 'null'];
    }
    return isBlob ? ['unchanged', 'null'] : ['unchanged', 'text', 'integer', 'real', 'null'];
  }

  function kindLabel(kind: ValueKind): string {
    switch (kind) {
      case 'default': {
        return m.workers_databases_row_kind_default();
      }
      case 'integer': {
        return m.workers_databases_row_kind_integer();
      }
      case 'null': {
        return m.workers_databases_row_kind_null();
      }
      case 'real': {
        return m.workers_databases_row_kind_real();
      }
      case 'text': {
        return m.workers_databases_row_kind_text();
      }
      case 'unchanged': {
        return m.workers_databases_row_kind_unchanged();
      }
    }
  }

  function setKind(index: number, value: string): void {
    const kind = kindsOf(index).find((known) => known === value);
    if (kind !== undefined) {
      request.fields[index].kind = kind;
    }
  }

  /** Whether a kind of value is written as a number. */
  function isNumber(kind: ValueKind): boolean {
    return NUMBERS.has(kind);
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
          <Field.Field data-invalid={problems[index] !== null ? true : undefined}>
            <Field.Label class="font-mono" for="database-row-{index}">
              <span class="wrap-anywhere" data-content dir="auto">{column.name}</span>
              {#if column.declaredType !== null}
                <span class="text-muted-foreground" data-content>{column.declaredType}</span>
              {/if}
            </Field.Label>
            {#if generated.has(column.name)}
              <p class="text-sm text-muted-foreground">{m.workers_databases_row_kind_unchanged()}</p>
            {:else}
              <div class="flex flex-col gap-2 sm:flex-row">
                <Select.Root type="single" value={field.kind} onValueChange={(value: string) => setKind(index, value)}>
                  <Select.Trigger
                    class="w-full sm:w-48"
                    aria-label={m.workers_databases_row_kind({ column: column.name })}
                  >
                    {kindLabel(field.kind)}
                  </Select.Trigger>
                  <Select.Content>
                    {#each kindsOf(index) as kind (kind)}
                      <Select.Item label={kindLabel(kind)} value={kind}>{kindLabel(kind)}</Select.Item>
                    {/each}
                  </Select.Content>
                </Select.Root>
                {#if field.kind === 'text'}
                  <Textarea
                    id="database-row-{index}"
                    class="min-h-9 font-mono"
                    dir="auto"
                    rows={1}
                    spellcheck="false"
                    bind:value={field.text}
                  />
                {:else if isNumber(field.kind)}
                  <Input
                    id="database-row-{index}"
                    class="font-mono"
                    aria-invalid={problems[index] !== null}
                    inputmode="decimal"
                    spellcheck="false"
                    bind:value={field.text}
                  />
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
