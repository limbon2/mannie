<script lang="ts">
  import CitationButton from '#lib/components/app/chat/CitationButton.svelte';
  import { getCitations, hasCitations } from '#lib/state/citations.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';

  // A cite link of a reply, such as [Guide p. 3](cite:notebooks/<key>)
  // (ADR-0049, decision 4): a button that shows the excerpt that a message of
  // the conversation holds under that key, or that opens a passage that the
  // model read with a tool where it comes from. A link whose provider has no
  // kind of context and no stored excerpt, or one in text that cites nothing,
  // such as a reasoning's, stays the link's text.
  let { key, label, provider }: { key: string; label: string; provider: string } = $props();

  const citations = hasCitations() ? getCitations() : null;
  const slots = getShellSlots();

  const cited = $derived(citations?.find(provider, key));
  const kind = $derived(slots.contextKinds.find(provider));
  const citesAny = $derived(citations !== null && (cited !== undefined || kind !== undefined));
</script>

{#if citesAny}
  <CitationButton {cited} {key} {kind} {label} />
{:else}
  {label}
{/if}
