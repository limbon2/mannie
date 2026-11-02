<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { Checkbox } from '@mannie/plugin-sdk/components/checkbox';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';

  import { m } from '../generated/messages.js';
  import { databaseActions } from '../state/database-actions.svelte.js';
  import { databasesFailureMessage } from '../utils/databases-messages.js';

  // Adds a database (plugins/workers/docs/databases.md, registering a
  // database): the owner may name it and mark it read-only. On this
  // computer, "Choose file…" opens the system's file dialog, where the owner
  // picks an existing SQLite file; on a worker, the owner types the file's
  // absolute path on the worker's host, which the worker checks (ADR-0057,
  // decision 3). The core or the worker checks the file and the name; a
  // refusal keeps the dialog open with the reason, and closing the file
  // dialog keeps it open too. A worker's name is content.
  const request = $derived(databaseActions.adding);
  const worker = $derived(request?.worker ?? null);
  const pathMissing = $derived(worker !== null && request !== null && request.path.trim() === '');

  function submit(event: SubmitEvent): void {
    event.preventDefault();
    if (request === null) {
      return;
    }
    const canAdd = !request.adding && !pathMissing;
    if (canAdd) {
      void databaseActions.confirmAdd(request);
    }
  }
</script>

<Dialog.Root bind:open={databaseActions.addOpen}>
  <Dialog.Content showCloseButton={false}>
    {#if request !== null}
      <form class="flex flex-col gap-4" data-add-database-dialog="" onsubmit={submit}>
        <Dialog.Header>
          {#if worker === null}
            <Dialog.Title>{m.workers_databases_add_title()}</Dialog.Title>
            <Dialog.Description>{m.workers_databases_add_description()}</Dialog.Description>
          {:else}
            <Dialog.Title class="wrap-anywhere" data-content dir="auto">
              {m.workers_databases_add_worker_title({ worker: worker.name })}
            </Dialog.Title>
            <Dialog.Description>{m.workers_databases_add_worker_description()}</Dialog.Description>
          {/if}
        </Dialog.Header>
        <Field.Group>
          {#if worker !== null}
            <Field.Field>
              <Field.Label for="add-database-path">{m.workers_databases_add_path()}</Field.Label>
              <Input
                id="add-database-path"
                class="font-mono"
                autocomplete="off"
                dir="ltr"
                required
                spellcheck="false"
                bind:value={request.path}
              />
              <Field.Description>{m.workers_databases_add_path_hint()}</Field.Description>
            </Field.Field>
          {/if}
          <Field.Field>
            <Field.Label for="add-database-name">{m.workers_databases_add_name()}</Field.Label>
            <Input id="add-database-name" autocomplete="off" spellcheck="false" bind:value={request.name} />
            <Field.Description>{m.workers_databases_add_name_hint()}</Field.Description>
          </Field.Field>
          <Field.Field orientation="horizontal">
            <Checkbox id="add-database-read-only" bind:checked={request.readOnly} />
            <Field.Content>
              <Field.Label for="add-database-read-only">{m.workers_databases_add_read_only()}</Field.Label>
              <Field.Description>{m.workers_databases_add_read_only_hint()}</Field.Description>
            </Field.Content>
          </Field.Field>
        </Field.Group>
        {#if request.failure !== null}
          <p class="text-sm text-destructive" data-add-database-failure="" role="alert">
            {databasesFailureMessage(request.failure)}
          </p>
        {/if}
        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={() => (databaseActions.addOpen = false)}>
            {m.workers_databases_cancel()}
          </Button>
          <Button data-add-database-choose="" disabled={request.adding || pathMissing} type="submit">
            {#if request.adding}
              <Spinner data-icon="inline-start" />
            {/if}
            {worker === null ? m.workers_databases_add_choose() : m.workers_databases_add_confirm()}
          </Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>
