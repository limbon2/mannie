<script lang="ts">
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import type { PartRendererProps } from '#lib/types/shell-slots.js';
  import { mentionIcon, mentionSummary, mentionVariantName } from '#lib/utils/mention-kinds.js';

  // Something that the owner's message mentioned, below the message
  // (docs/features/chat/mentions.md): its kind's icon, its label and what the
  // model got of it. The text that the model read stays out of the view.
  let { part }: PartRendererProps = $props();

  const slots = getShellSlots();

  /** The mention with the words of its kind, or `null` for a part of another kind. */
  const shown = $derived.by(() => {
    if (part.content.kind !== 'mention') {
      return null;
    }
    const { mention } = part.content;
    const kind = slots.mentionKinds.find(mention.provider);
    return {
      Icon: mentionIcon(kind, mention.variant),
      label: mention.label,
      provider: mention.provider,
      summary: mentionSummary(kind, mention.summary),
      variantName: mentionVariantName(kind, mention.variant),
    };
  });
</script>

{#if shown !== null}
  <div
    class="ms-auto flex w-fit max-w-[85%] min-w-0 items-center gap-1.5 rounded-md border bg-muted/40 px-2 py-1 text-xs"
    data-mention={shown.provider}
  >
    <shown.Icon class="size-3.5 shrink-0 text-muted-foreground" aria-hidden="true" />
    <span class="sr-only">{shown.variantName}</span>
    <span class="min-w-0 truncate font-mono" data-content>{shown.label}</span>
    {#if shown.summary !== null}
      <span class="shrink-0 text-muted-foreground">{shown.summary}</span>
    {/if}
  </div>
{/if}
