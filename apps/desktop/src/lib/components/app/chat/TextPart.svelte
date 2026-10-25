<script lang="ts">
  import Markdown from '#lib/components/app/markdown/Markdown.svelte';
  import { m } from '#lib/paraglide/messages.js';
  import { setCitations } from '#lib/state/citations.js';
  import { getConversationContext, hasConversationContext } from '#lib/state/conversation-context.js';
  import type { PartRendererProps } from '#lib/types/shell-slots.js';

  // The reply's text, as Markdown (ADR-0021). Text that ended early says so.
  // Its cite links name the excerpts that the conversation's messages hold,
  // or passages that the model read with a tool (ADR-0049, decision 4).
  let { part }: PartRendererProps = $props();

  const conversation = hasConversationContext() ? getConversationContext() : null;

  const text = $derived(part.content.kind === 'text' ? part.content.text : '');
  const streaming = $derived(part.state === 'streaming');

  setCitations({ find: (provider, key) => conversation?.cited(provider, key) });
</script>

<div class="flex flex-col gap-1">
  <Markdown {streaming} {text} />
  {#if part.state === 'incomplete'}
    <p class="text-xs text-muted-foreground">{m.chat_part_incomplete()}</p>
  {/if}
</div>
