<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';

  import * as AlertDialog from '#lib/components/ui/alert-dialog/index.js';
  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatActions } from '#lib/state/chat-actions.svelte.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';

  // Confirms the deletion of the conversation that `chatActions.startDelete`
  // opened it for. A reply that is being written stops first.
  const request = $derived(chatActions.deleting);
</script>

<AlertDialog.Root bind:open={chatActions.deleteOpen}>
  <AlertDialog.Content>
    {#if request !== null}
      <AlertDialog.Header>
        <AlertDialog.Title>{m.chat_delete_title()}</AlertDialog.Title>
        <AlertDialog.Description>
          {m.chat_delete_description({ title: request.conversation.title })}
        </AlertDialog.Description>
      </AlertDialog.Header>
      {#if request.failure !== null}
        <Alert.Root variant="destructive">
          <CircleAlertIcon />
          <Alert.Description>{chatFailureMessage(request.failure)}</Alert.Description>
        </Alert.Root>
      {/if}
      <AlertDialog.Footer>
        <AlertDialog.Cancel>{m.chat_cancel()}</AlertDialog.Cancel>
        <Button
          disabled={request.deleting}
          variant="destructive"
          onclick={() => void chatActions.confirmDelete(request)}
        >
          {m.chat_delete()}
        </Button>
      </AlertDialog.Footer>
    {/if}
  </AlertDialog.Content>
</AlertDialog.Root>
