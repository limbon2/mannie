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
  // database): the owner may name it and mark it read-only, and "Choose
  // file…" opens the system's file dialog, where the owner picks an existing
  // SQLite file on this computer. The core checks the file and the name; a
  // refusal keeps the dialog open with the reason, and closing the file
  // dialog keeps it open too.
  const request = $derived(databaseActions.adding);

  function choose(event: SubmitEvent): void {
    event.preventDefault();
    if (request === null) {
      return;
    }
    if (!request.adding) {
      void databaseActions.confirmAdd(request);
    }
  }
</script>

<Dialog.Root bind:open={databaseActions.addOpen}>
  <Dialog.Content showCloseButton={false}>
    {#if request !== null}
      <form class="flex flex-col gap-4" data-add-database-dialog="" onsubmit={choose}>
        <Dialog.Header>
          <Dialog.Title>{m.workers_databases_add_title()}</Dialog.Title>
          <Dialog.Description>{m.workers_databases_add_description()}</Dialog.Description>
        </Dialog.Header>
        <Field.Group>
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
          <Button data-add-database-choose="" disabled={request.adding} type="submit">
            {#if request.adding}
              <Spinner data-icon="inline-start" />
            {/if}
            {m.workers_databases_add_choose()}
          </Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>
