<script lang="ts">
  import * as AlertDialog from '@mannie/plugin-sdk/components/alert-dialog';
  import { Button } from '@mannie/plugin-sdk/components/button';

  import { m } from '../generated/messages.js';
  import { databaseActions } from '../state/database-actions.svelte.js';
  import { databasesFailureMessage } from '../utils/databases-messages.js';

  // Asks before Mannie forgets a database (plugins/workers/docs/
  // databases.md, registering a database): the file stays as it is, and the
  // owner can add it again. The name is content.
  const request = $derived(databaseActions.removing);
</script>

<AlertDialog.Root bind:open={databaseActions.removeOpen}>
  <!-- The width needs the variant of the default size, which the component's own maximum uses. -->
  <AlertDialog.Content class="data-[size=default]:sm:max-w-md" data-remove-database-dialog="">
    {#if request !== null}
      <AlertDialog.Header>
        <AlertDialog.Title class="wrap-anywhere">
          {m.workers_databases_remove_title({ name: request.database.name })}
        </AlertDialog.Title>
        <AlertDialog.Description>{m.workers_databases_remove_description()}</AlertDialog.Description>
      </AlertDialog.Header>
      {#if request.failure !== null}
        <p class="text-sm text-destructive" role="alert">{databasesFailureMessage(request.failure)}</p>
      {/if}
      <AlertDialog.Footer>
        <AlertDialog.Cancel>{m.workers_databases_cancel()}</AlertDialog.Cancel>
        <Button
          data-remove-database-confirm=""
          disabled={request.removing}
          variant="destructive"
          onclick={() => void databaseActions.confirmRemove(request)}
        >
          {m.workers_databases_remove_confirm()}
        </Button>
      </AlertDialog.Footer>
    {/if}
  </AlertDialog.Content>
</AlertDialog.Root>
