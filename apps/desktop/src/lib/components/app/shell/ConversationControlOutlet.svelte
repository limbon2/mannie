<script lang="ts">
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import type { ConversationControlProps } from '#lib/types/shell-slots.js';

  // The controls that features add to a conversation's toolbar (ADR-0026),
  // in their order. The conversation view renders this outlet and never
  // imports the features behind the controls.
  let { backgroundCommands = [], conversation, turns }: ConversationControlProps = $props();

  const slots = getShellSlots();

  const controls = $derived([...slots.conversationControls.entries].sort((left, right) => left.order - right.order));
</script>

{#each controls as control (control.id)}
  <control.component {backgroundCommands} {conversation} {turns} />
{/each}
