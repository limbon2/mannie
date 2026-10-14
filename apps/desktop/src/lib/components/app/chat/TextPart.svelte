<script lang="ts">
  import Markdown from '#lib/components/app/markdown/Markdown.svelte';
  import { m } from '#lib/paraglide/messages.js';
  import type { PartRendererProps } from '#lib/types/shell-slots.js';

  // The reply's text, as Markdown (ADR-0021). Text that ended early says so.
  let { part }: PartRendererProps = $props();

  const text = $derived(part.content.kind === 'text' ? part.content.text : '');
  const streaming = $derived(part.state === 'streaming');
</script>

<div class="flex flex-col gap-1">
  <Markdown {streaming} {text} />
  {#if part.state === 'incomplete'}
    <p class="text-xs text-muted-foreground">{m.chat_part_incomplete()}</p>
  {/if}
</div>
