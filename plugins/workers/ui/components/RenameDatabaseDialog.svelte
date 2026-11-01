<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';

  import { m } from '../generated/messages.js';
  import { databaseActions } from '../state/database-actions.svelte.js';
  import { databasesFailureMessage } from '../utils/databases-messages.js';

  // Names a database (plugins/workers/docs/databases.md, registering a
  // database): the name is unique among the registered databases, and
  // agents use it. The core checks it; a refused one keeps the dialog open
  // with the reason. The current name is content.
  const request = $derived(databaseActions.renaming);

  function save(event: SubmitEvent): void {
    event.preventDefault();
    if (request === null) {
      return;
    }
    if (!request.saving) {
      void databaseActions.confirmRename(request);
    }
  }
</script>

<Dialog.Root bind:open={databaseActions.renameOpen}>
  <Dialog.Content showCloseButton={false}>
    {#if request !== null}
      <form class="flex flex-col gap-4" data-rename-database-dialog="" onsubmit={save}>
        <Dialog.Header>
          <Dialog.Title class="wrap-anywhere">
            {m.workers_databases_rename_title({ name: request.database.name })}
          </Dialog.Title>
        </Dialog.Header>
        <Field.Field data-invalid={request.failure !== null ? true : undefined}>
          <Field.Label for="rename-database-name">{m.workers_databases_add_name()}</Field.Label>
          <Input
            id="rename-database-name"
            aria-invalid={request.failure !== null}
            autocomplete="off"
            spellcheck="false"
            bind:value={request.name}
          />
          {#if request.failure !== null}
            <Field.Error>{databasesFailureMessage(request.failure)}</Field.Error>
          {/if}
        </Field.Field>
        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={() => (databaseActions.renameOpen = false)}>
            {m.workers_databases_cancel()}
          </Button>
          <Button data-rename-database-save="" disabled={request.saving} type="submit">
            {m.workers_databases_rename_save()}
          </Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>
