<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Dialog from '#lib/components/ui/dialog/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatActions } from '#lib/state/chat-actions.svelte.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';

  // The summary that stands in for older turns in the model's context
  // (docs/features/chat/conversations.md, context budget and compaction).
  // `chatActions.showCheckpoint` opens it.
  const shown = $derived(chatActions.checkpoint);
</script>

<Dialog.Root bind:open={chatActions.checkpointOpen}>
  <Dialog.Content class="sm:max-w-2xl" showCloseButton={false}>
    <Dialog.Header>
      <Dialog.Title>{m.chat_checkpoint_title()}</Dialog.Title>
      <Dialog.Description>
        {#if shown.status === 'loaded'}
          {m.chat_checkpoint_description({
            after: shown.checkpoint.view.tokensAfter,
            before: shown.checkpoint.view.tokensBefore,
            turns: shown.checkpoint.view.coversThroughTurn + 1,
          })}
        {:else}
          {m.chat_checkpoint_description_loading()}
        {/if}
      </Dialog.Description>
    </Dialog.Header>
    {#if shown.status === 'failed'}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Description>{chatFailureMessage(shown.failure)}</Alert.Description>
      </Alert.Root>
    {:else if shown.status === 'loading'}
      <p class="flex items-center gap-2 text-sm text-muted-foreground">
        <Spinner aria-label={m.chat_loading()} />
        <span>{m.chat_loading()}</span>
      </p>
    {:else}
      <div class="max-h-[60vh] overflow-y-auto rounded-md border p-3 text-sm whitespace-pre-wrap" data-content>
        {shown.checkpoint.summary}
      </div>
    {/if}
    <Dialog.Footer>
      <Button variant="outline" onclick={() => (chatActions.checkpointOpen = false)}>{m.chat_close()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
