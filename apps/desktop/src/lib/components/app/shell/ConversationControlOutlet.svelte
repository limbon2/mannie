<script lang="ts">
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import type { ConversationControlPlace, ConversationControlProps } from '#lib/types/shell-slots.js';
  import { controlsIn } from '#lib/utils/conversation-controls.js';

  // The controls that features add to a conversation (ADR-0026), those of
  // one place in their order (ADR-0042): the toolbar's, the entries of the
  // conversation's menu, or the message box's. The conversation view renders
  // this outlet and never imports the features behind the controls.
  interface Props extends ConversationControlProps {
    /** The place whose controls the outlet shows. */
    readonly place?: ConversationControlPlace;
  }

  let { backgroundCommands = [], conversation, opener, place = 'toolbar', turns }: Props = $props();

  const slots = getShellSlots();

  const controls = $derived(controlsIn(slots.conversationControls.entries, place));
</script>

{#each controls as control (control.id)}
  <control.component {backgroundCommands} {conversation} {opener} {turns} />
{/each}
