<script lang="ts">
  import * as AlertDialog from '@mannie/plugin-sdk/components/alert-dialog';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import { Textarea } from '@mannie/plugin-sdk/components/textarea';
  import PlayIcon from '@mannie/plugin-sdk/icons/play';
  import SquareIcon from '@mannie/plugin-sdk/icons/square';

  import type { DatabaseView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { sqlSession } from '../state/database-sql.svelte.js';
  import { databasesFailureMessage } from '../utils/databases-messages.js';

  import DatabaseGrid from './DatabaseGrid.svelte';

  // The SQL tab (plugins/workers/docs/databases.md, the view): one statement
  // that the owner writes, which Run or Ctrl+Enter runs. A read shows its
  // rows and Stop ends it; a write says how many rows it changed; a change
  // of the schema runs after a confirmation that names it. The tab keeps the
  // session's statements. Statements and values are content.
  let { database, onschemachange }: { database: DatabaseView; onschemachange: () => void } = $props();

  const session = $derived(sqlSession(database.key));
  const confirmOpen = $derived(session.confirming !== null);
  const resultRows = $derived(session.result?.rows ?? null);

  async function run(): Promise<void> {
    if (await session.start(database)) {
      onschemachange();
    }
  }

  async function confirm(): Promise<void> {
    if (await session.confirmSchema(database)) {
      onschemachange();
    }
  }

  function keydown(event: KeyboardEvent): void {
    const runs = event.key === 'Enter' && (event.ctrlKey || event.metaKey);
    if (runs) {
      event.preventDefault();
      void run();
    }
  }

  function closeConfirmation(open: boolean): void {
    if (!open) {
      session.confirming = null;
    }
  }
</script>

<div class="flex flex-col gap-4" data-database-sql="">
  {#if database.readOnly}
    <p class="text-sm text-muted-foreground">{m.workers_databases_sql_read_only()}</p>
  {/if}
  <Field.Field>
    <Field.Label for="database-sql-statement">{m.workers_databases_sql_label()}</Field.Label>
    <Textarea
      id="database-sql-statement"
      class="min-h-32 font-mono"
      aria-describedby="database-sql-shortcut"
      dir="ltr"
      placeholder={m.workers_databases_sql_placeholder()}
      spellcheck="false"
      bind:value={session.text}
      onkeydown={keydown}
    />
    <Field.Description id="database-sql-shortcut">{m.workers_databases_sql_shortcut()}</Field.Description>
  </Field.Field>
  <div class="flex flex-wrap items-center gap-2">
    <Button
      data-database-sql-run=""
      disabled={session.running !== null || session.text.trim() === ''}
      onclick={() => void run()}
    >
      <PlayIcon data-icon="inline-start" />
      {m.workers_databases_sql_run()}
    </Button>
    {#if session.running === 'reading'}
      <Button data-database-sql-stop="" variant="outline" onclick={() => void session.stop()}>
        <SquareIcon data-icon="inline-start" />
        {m.workers_databases_sql_stop()}
      </Button>
    {/if}
    {#if session.running !== null}
      <span class="flex items-center gap-2 text-sm text-muted-foreground" aria-live="polite">
        <Spinner />
        {m.workers_databases_sql_running()}
      </span>
    {/if}
  </div>

  {#if session.failure !== null}
    <p class="text-sm text-destructive" data-database-sql-failure="" role="alert">
      {databasesFailureMessage(session.failure)}
    </p>
  {:else if session.result !== null}
    <section class="flex flex-col gap-2" aria-live="polite" data-database-sql-result="">
      {#if session.result.kind === 'changed'}
        <p class="text-sm" data-database-sql-changed="">
          {m.workers_databases_sql_changed({ count: Number(session.result.changes) })}
        </p>
      {/if}
      {#if resultRows !== null}
        {#if resultRows.columns.length === 0}
          <p class="text-sm text-muted-foreground">{m.workers_databases_sql_no_columns()}</p>
        {:else}
          <p class="text-sm text-muted-foreground">
            {m.workers_databases_sql_rows({ count: resultRows.rows.length })}
          </p>
          <DatabaseGrid columns={resultRows.columns} rows={resultRows.rows} />
          {#if resultRows.cut === 'rows'}
            <p class="text-sm text-muted-foreground">{m.workers_databases_sql_cut_rows()}</p>
          {:else if resultRows.cut === 'bytes'}
            <p class="text-sm text-muted-foreground">{m.workers_databases_sql_cut_bytes()}</p>
          {/if}
        {/if}
      {/if}
    </section>
  {/if}

  {#if session.history.length > 0}
    <section class="flex flex-col gap-2">
      <h2 class="text-sm font-semibold">{m.workers_databases_sql_history()}</h2>
      <ul class="flex flex-col gap-1">
        {#each session.history as statement (statement)}
          <li class="flex items-start gap-2">
            <code class="min-w-0 flex-1 truncate text-xs" data-content dir="ltr">{statement}</code>
            <Button size="xs" variant="ghost" onclick={() => session.use(statement)}>
              {m.workers_databases_sql_history_use()}
            </Button>
          </li>
        {/each}
      </ul>
    </section>
  {/if}
</div>

<AlertDialog.Root open={confirmOpen} onOpenChange={closeConfirmation}>
  <AlertDialog.Content data-database-schema-dialog="">
    <AlertDialog.Header>
      <AlertDialog.Title>{m.workers_databases_sql_schema_title()}</AlertDialog.Title>
      <AlertDialog.Description>
        {m.workers_databases_sql_schema_description({ name: database.name })}
      </AlertDialog.Description>
    </AlertDialog.Header>
    {#if session.confirming !== null}
      <pre
        class="max-h-48 overflow-auto rounded-md bg-muted px-3 py-2 font-mono text-xs whitespace-pre-wrap"
        data-content
        dir="ltr">{session.confirming}</pre>
    {/if}
    <AlertDialog.Footer>
      <AlertDialog.Cancel>{m.workers_databases_cancel()}</AlertDialog.Cancel>
      <Button data-database-schema-confirm="" variant="destructive" onclick={() => void confirm()}>
        {m.workers_databases_sql_schema_confirm()}
      </Button>
    </AlertDialog.Footer>
  </AlertDialog.Content>
</AlertDialog.Root>
