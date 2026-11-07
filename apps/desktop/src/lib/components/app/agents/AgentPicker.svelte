<script lang="ts">
  import { commands } from '#lib/generated/bindings.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import type { ConversationControlProps } from '#lib/types/shell-slots.js';
  import { call } from '#lib/utils/calls.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';

  import AgentMenu from './AgentMenu.svelte';

  // The agent of a conversation's next replies, in its message box (ADR-0059,
  // decision 6). A task conversation's agent never changes, so it has no picker.
  let { conversation }: ConversationControlProps = $props();

  let failure = $state<ChatFailure | null>(null);

  const isTask = $derived(conversation.task !== null);

  async function choose(key: string | null): Promise<void> {
    const outcome = await call(() => commands.setConversationAgent({ agentKey: key, conversationId: conversation.id }));
    failure = outcome.ok ? null : outcome.failure;
  }
</script>

{#if !isTask}
  <AgentMenu chosen={conversation.agentKey ?? null} onChoose={(key: string | null) => void choose(key)} />
  {#if failure !== null}
    <span class="text-sm text-destructive" role="alert">{chatFailureMessage(failure)}</span>
  {/if}
{/if}
