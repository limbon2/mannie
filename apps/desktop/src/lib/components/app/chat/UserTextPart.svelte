<script lang="ts">
  import type { PartRendererProps } from '#lib/types/shell-slots.js';

  // The owner's message, as the owner wrote it: plain text with its line
  // breaks.
  let { part }: PartRendererProps = $props();

  const text = $derived(part.content.kind === 'user_text' ? part.content.text : '');
</script>

<div class="ms-auto w-fit max-w-[85%] rounded-lg bg-muted px-3 py-2 text-sm whitespace-pre-wrap" data-content>
  {text}
</div>
