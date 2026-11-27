<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Dialog from '#lib/components/ui/dialog/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import type { CheckpointView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatActions } from '#lib/state/chat-actions.svelte.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';

  // The summary that stands in for older turns in the model's context
  // (docs/features/chat/conversations.md, context budget and compaction).
  // `chatActions.showCheckpoint` opens it.
  const shown = $derived(chatActions.checkpoint);
  const note = $derived(shown.status === 'loaded' ? triggerNote(shown.checkpoint.view.trigger) : null);

  function checkpointDescription(view: CheckpointView): string {
    if ((view.coversThroughPart ?? null) !== null) {
      return m.chat_checkpoint_description_step({
        after: view.tokensAfter,
        before: view.tokensBefore,
        turn: view.coversThroughTurn + 1,
      });
    }
    return m.chat_checkpoint_description({
      after: view.tokensAfter,
      before: view.tokensBefore,
      turns: view.coversThroughTurn + 1,
    });
  }

  function triggerNote(trigger: string | null | undefined): string | null {
    if (trigger === null) {
      return null;
    }
    if (trigger === undefined) {
      return null;
    }

    switch (trigger) {
      case 'automatic': {
        return m.chat_checkpoint_trigger_automatic();
      }
      case 'manual': {
        return m.chat_checkpoint_trigger_manual();
      }
      case 'overflow': {
        return m.chat_checkpoint_trigger_overflow();
      }
      default: {
        return null;
      }
    }
  }
</script>

<Dialog.Root bind:open={chatActions.checkpointOpen}>
  <Dialog.Content class="sm:max-w-2xl" showCloseButton={false}>
    <Dialog.Header>
      <Dialog.Title>{m.chat_checkpoint_title()}</Dialog.Title>

      <Dialog.Description>
        {#if shown.status === 'loaded'}
          {checkpointDescription(shown.checkpoint.view)}
        {:else}
          {m.chat_checkpoint_description_loading()}
        {/if}
      </Dialog.Description>

      {#if note !== null}
        <p class="text-xs text-muted-foreground">{note}</p>
      {/if}
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
