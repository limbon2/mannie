<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import * as AlertDialog from '@mannie/plugin-sdk/components/alert-dialog';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';

  import { m } from '../generated/messages.js';
  import { notebookActions } from '../state/notebook-actions.svelte.js';
  import { notebooksFailureMessage } from '../utils/notebooks-messages.js';

  // Confirms the removal of a source (plugins/notebooks/docs/README.md,
  // sources): the plugin deletes its text and its passages from the store.
  // The chats that got its passages keep them in their transcripts.
  const request = $derived(notebookActions.removing);
</script>

<AlertDialog.Root bind:open={notebookActions.removeOpen}>
  <!-- The width needs the variant of the default size, which the component's own maximum uses. -->
  <AlertDialog.Content class="data-[size=default]:sm:max-w-md">
    {#if request !== null}
      <AlertDialog.Header>
        <AlertDialog.Title>{m.notebooks_remove_title({ name: request.source.name })}</AlertDialog.Title>
        <AlertDialog.Description>{m.notebooks_remove_description()}</AlertDialog.Description>
      </AlertDialog.Header>
      <div class="text-sm" role="status">
        {#if request.removing}
          <span>{m.notebooks_removing()}</span>
        {/if}
      </div>
      {#if request.failure !== null}
        <Alert.Root variant="destructive">
          <CircleAlertIcon />
          <Alert.Title>{m.notebooks_remove_failed_title()}</Alert.Title>
          <Alert.Description>{notebooksFailureMessage(request.failure)}</Alert.Description>
        </Alert.Root>
      {/if}
      <AlertDialog.Footer>
        <AlertDialog.Cancel>{m.notebooks_cancel()}</AlertDialog.Cancel>
        <Button
          data-notebooks-remove-confirm=""
          disabled={request.removing}
          variant="destructive"
          onclick={() => void notebookActions.confirmRemove(request)}
        >
          {m.notebooks_remove_confirm()}
        </Button>
      </AlertDialog.Footer>
    {/if}
  </AlertDialog.Content>
</AlertDialog.Root>
