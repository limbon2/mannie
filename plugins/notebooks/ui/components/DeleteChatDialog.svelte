<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import * as AlertDialog from '@mannie/plugin-sdk/components/alert-dialog';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';

  import { m } from '../generated/messages.js';
  import { notebookActions } from '../state/notebook-actions.svelte.js';

  // Confirms the deletion of one of a notebook's chats (plugins/notebooks/
  // docs/README.md, chats): the core deletes the chat with its replies, and
  // the notebook and its sources stay. The chat's title is content.
  const request = $derived(notebookActions.chatDeleting);
</script>

<AlertDialog.Root bind:open={notebookActions.chatDeleteOpen}>
  <!-- The width needs the variant of the default size, which the component's own maximum uses. -->
  <AlertDialog.Content class="data-[size=default]:sm:max-w-md" data-notebooks-chat-delete-dialog="">
    {#if request !== null}
      <AlertDialog.Header>
        <AlertDialog.Title class="wrap-anywhere">
          {m.notebooks_chat_delete_title({ title: request.chat.title })}
        </AlertDialog.Title>
        <AlertDialog.Description>{m.notebooks_chat_delete_description()}</AlertDialog.Description>
      </AlertDialog.Header>
      <div class="text-sm" role="status">
        {#if request.deleting}
          <span>{m.notebooks_chat_deleting()}</span>
        {/if}
      </div>
      {#if request.failure !== null}
        <Alert.Root variant="destructive">
          <CircleAlertIcon />
          <Alert.Title>{m.notebooks_chat_delete_failed_title()}</Alert.Title>
          <Alert.Description>{request.failure}</Alert.Description>
        </Alert.Root>
      {/if}
      <AlertDialog.Footer>
        <AlertDialog.Cancel>{m.notebooks_cancel()}</AlertDialog.Cancel>
        <Button
          data-notebooks-chat-delete-confirm=""
          disabled={request.deleting}
          variant="destructive"
          onclick={() => void notebookActions.confirmDeleteChat(request)}
        >
          {m.notebooks_delete_confirm()}
        </Button>
      </AlertDialog.Footer>
    {/if}
  </AlertDialog.Content>
</AlertDialog.Root>
