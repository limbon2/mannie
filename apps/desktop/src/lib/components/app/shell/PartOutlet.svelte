<script lang="ts">
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import type { PartRendererProps } from '#lib/types/shell-slots.js';
  import { partKind } from '#lib/utils/parts.js';

  import GenericPart from './GenericPart.svelte';

  let { conversationId, latest, part }: PartRendererProps = $props();

  const slots = getShellSlots();

  // A kind that no feature registered still renders, as a generic card.
  const kind = $derived(partKind(part));
  const renderer = $derived(slots.partRenderers.find(kind));
</script>

<!-- The wrapper takes no box of its own; its attributes name the part, its
     kind and its state for styling, debugging, the end-to-end tests and the
     jump to a part that waits for the owner. -->
<div class="contents" data-part-id={part.id} data-part-kind={kind} data-part-state={part.state}>
  {#if renderer}
    <renderer.component {conversationId} {latest} {part} />
  {:else}
    <GenericPart {conversationId} {latest} {part} />
  {/if}
</div>
