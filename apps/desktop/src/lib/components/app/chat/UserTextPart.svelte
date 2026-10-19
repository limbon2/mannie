<script lang="ts">
  import type { PartRendererProps } from '#lib/types/shell-slots.js';

  // The owner's message, as the owner wrote it: plain text with its line
  // breaks. A message with only images has no text, and shows only them
  // (plugins/images/docs/README.md, the transcript).
  let { part }: PartRendererProps = $props();

  const text = $derived(part.content.kind === 'user_text' ? part.content.text : '');
</script>

{#if text.trim() !== ''}
  <div class="ms-auto w-fit max-w-[85%] rounded-lg bg-muted px-3 py-2 text-sm whitespace-pre-wrap" data-content>
    {text}
  </div>
{/if}
