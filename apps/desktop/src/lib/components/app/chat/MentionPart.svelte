<script lang="ts">
  import * as Attachment from '#lib/components/ui/attachment/index.js';
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
  <Attachment.Root class="ms-auto max-w-[85%] min-w-0" data-mention={shown.provider} size="xs">
    <Attachment.Media>
      <shown.Icon />
    </Attachment.Media>
    <Attachment.Content>
      <span class="sr-only">{shown.variantName}</span>
      <Attachment.Title class="font-mono" data-content>{shown.label}</Attachment.Title>
      {#if shown.summary !== null}
        <Attachment.Description>{shown.summary}</Attachment.Description>
      {/if}
    </Attachment.Content>
  </Attachment.Root>
{/if}
