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

<!-- The wrapper takes no box of its own; its attributes name the part's kind
     and state for styling, debugging and the end-to-end tests. -->
<div class="contents" data-part-kind={kind} data-part-state={part.state}>
  {#if renderer}
    <renderer.component {conversationId} {latest} {part} />
  {:else}
    <GenericPart {conversationId} {latest} {part} />
  {/if}
</div>
