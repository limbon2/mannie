<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import * as AlertDialog from '@mannie/plugin-sdk/components/alert-dialog';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import TriangleAlertIcon from '@mannie/plugin-sdk/icons/triangle-alert';

  import { m } from '../generated/messages.js';
  import { notebookActions } from '../state/notebook-actions.svelte.js';
  import { notebooksFailureMessage } from '../utils/notebooks-messages.js';

  // Confirms the deletion of a notebook (plugins/notebooks/docs/README.md,
  // notebooks): it says how many of the notebook's own chats go with it and
  // how many other chats use it, deletes its chats, then its sources with
  // their text and passages, and takes it from the other chats, which keep
  // their messages (ADR-0051, decision 6). A chat that stayed says why.
  const request = $derived(notebookActions.deleting);
</script>

<AlertDialog.Root bind:open={notebookActions.deleteOpen}>
  <!-- The width needs the variant of the default size, which the component's own maximum uses. -->
  <AlertDialog.Content class="data-[size=default]:sm:max-w-md">
    {#if request !== null}
      <AlertDialog.Header>
        <AlertDialog.Title>{m.notebooks_delete_title({ name: request.notebook.name })}</AlertDialog.Title>
        <AlertDialog.Description>
          {m.notebooks_delete_description()}
          {#if request.ownChats > 0}
            {m.notebooks_delete_own_chats({ count: request.ownChats })}
          {/if}
          {#if request.usingChats > 0}
            {m.notebooks_delete_using_chats({ count: request.usingChats })}
          {/if}
        </AlertDialog.Description>
      </AlertDialog.Header>
      <div class="text-sm" role="status">
        {#if request.deleting}
          <span>{m.notebooks_deleting()}</span>
        {/if}
      </div>
      {#if request.failure !== null}
        <Alert.Root variant="destructive">
          <CircleAlertIcon />
          <Alert.Title>{m.notebooks_delete_failed_title()}</Alert.Title>
          <Alert.Description>{notebooksFailureMessage(request.failure)}</Alert.Description>
        </Alert.Root>
      {/if}
      {#if request.chatFailure !== null}
        <Alert.Root data-notebooks-chat-delete-failed="" variant="destructive">
          <CircleAlertIcon />
          <Alert.Title>{m.notebooks_delete_failed_title()}</Alert.Title>
          <Alert.Description>{m.notebooks_delete_chat_failed({ reason: request.chatFailure })}</Alert.Description>
        </Alert.Root>
      {/if}
      {#if request.detachFailure !== null}
        <Alert.Root data-notebooks-detach-failed="">
          <TriangleAlertIcon />
          <Alert.Title>{m.notebooks_detach_failed_title()}</Alert.Title>
          <Alert.Description>{request.detachFailure}</Alert.Description>
        </Alert.Root>
      {/if}
      <AlertDialog.Footer>
        {#if request.detachFailure === null}
          <AlertDialog.Cancel>{m.notebooks_cancel()}</AlertDialog.Cancel>
          <Button
            data-notebooks-delete-confirm=""
            disabled={request.deleting}
            variant="destructive"
            onclick={() => void notebookActions.confirmDelete(request)}
          >
            {m.notebooks_delete_confirm()}
          </Button>
        {:else}
          <AlertDialog.Cancel>{m.notebooks_close()}</AlertDialog.Cancel>
        {/if}
      </AlertDialog.Footer>
    {/if}
  </AlertDialog.Content>
</AlertDialog.Root>
