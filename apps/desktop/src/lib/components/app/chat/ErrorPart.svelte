<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import RotateCcwIcon from '@lucide/svelte/icons/rotate-ccw';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatActions } from '#lib/state/chat-actions.svelte.js';
  import { getConversationContext } from '#lib/state/conversation-context.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import type { PartRendererProps } from '#lib/types/shell-slots.js';
  import { chatFailureMessage, runFailureMessage } from '#lib/utils/chat-errors.js';

  // A failed reply: what failed, what the owner can do, and Retry for the
  // latest turn, unless the conversation shows read-only.
  let { conversationId, latest, part }: PartRendererProps = $props();

  const context = getConversationContext();

  let retrying = $state(false);
  let retryFailure = $state<ChatFailure | null>(null);

  const message = $derived(part.content.kind === 'error' ? runFailureMessage(part.content.failure) : '');

  async function retry(): Promise<void> {
    retrying = true;
    retryFailure = await chatActions.retry(conversationId);
    retrying = false;
  }
</script>

<Alert.Root variant="destructive">
  <CircleAlertIcon />
  <Alert.Title>{m.chat_reply_failed()}</Alert.Title>
  <Alert.Description>
    <p>{message}</p>
    {#if retryFailure !== null}
      <p>{chatFailureMessage(retryFailure)}</p>
    {/if}
  </Alert.Description>
  {#if latest && !context.readOnly}
    <Alert.Action>
      <Button disabled={retrying} size="sm" variant="outline" onclick={retry}>
        <RotateCcwIcon data-icon="inline-start" />
        {m.chat_retry()}
      </Button>
    </Alert.Action>
  {/if}
</Alert.Root>
