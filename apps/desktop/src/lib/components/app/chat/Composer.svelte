<script lang="ts">
  import SendIcon from '@lucide/svelte/icons/send';
  import SquareIcon from '@lucide/svelte/icons/square';

  import { Button } from '#lib/components/ui/button/index.js';
  import { Textarea } from '#lib/components/ui/textarea/index.js';
  import * as Tooltip from '#lib/components/ui/tooltip/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';

  // The message box (docs/features/chat/conversation-view.md, the message
  // box): Enter sends, Shift+Enter starts a new line, and Escape stops the
  // reply while the box has the focus. The text stays when sending fails.
  interface Props {
    /** Whether the box takes no message, such as without a provider profile. */
    disabled?: boolean;
    /** Sends a message; resolves to why it failed, or `null`. */
    onSend: (text: string) => Promise<ChatFailure | null>;
    /** Stops the reply that is being written, when there can be one. */
    onStop?: () => Promise<void>;
    /** Whether a reply is being written, which the button then stops. */
    replying?: boolean;
  }

  let { disabled = false, onSend, onStop, replying = false }: Props = $props();

  let text = $state('');
  let sending = $state(false);
  let failure = $state<ChatFailure | null>(null);

  const hasText = $derived(text.trim() !== '');
  const canSend = $derived(hasText && !disabled && !sending && !replying);

  async function send(): Promise<void> {
    if (!canSend) {
      return;
    }
    sending = true;
    failure = null;
    const result = await onSend(text);
    sending = false;
    if (result === null) {
      text = '';
    } else {
      failure = result;
    }
  }

  function handleKeydown(event: KeyboardEvent): void {
    const sends = event.key === 'Enter' && !event.shiftKey && !event.isComposing;
    if (sends) {
      event.preventDefault();
      void send();
      return;
    }
    const stops = event.key === 'Escape' && replying;
    if (stops) {
      event.preventDefault();
      void onStop?.();
    }
  }

  function handleSubmit(event: SubmitEvent): void {
    event.preventDefault();
    void send();
  }
</script>

<form class="flex flex-col gap-2 border-t bg-background p-3" onsubmit={handleSubmit}>
  <div class="flex items-end gap-2">
    <Textarea
      class="max-h-48 min-h-10 resize-none"
      aria-describedby="composer-hint"
      aria-label={m.chat_composer_label()}
      {disabled}
      placeholder={m.chat_composer_placeholder()}
      rows={1}
      bind:value={text}
      onkeydown={handleKeydown}
    />
    {#if replying}
      <Button type="button" variant="outline" onclick={() => void onStop?.()}>
        <SquareIcon data-icon="inline-start" />
        {m.chat_stop()}
      </Button>
    {:else}
      <Tooltip.Root>
        <Tooltip.Trigger disabled={!canSend} type="submit">
          {#snippet child({ props })}
            <Button {...props} aria-label={m.chat_send()} size="icon">
              <SendIcon />
            </Button>
          {/snippet}
        </Tooltip.Trigger>
        <Tooltip.Content>{m.chat_send()}</Tooltip.Content>
      </Tooltip.Root>
    {/if}
  </div>
  {#if failure !== null}
    <p class="text-sm text-destructive" role="alert">{chatFailureMessage(failure)}</p>
  {/if}
  <p id="composer-hint" class="text-xs text-muted-foreground">{m.chat_composer_hint()}</p>
</form>
