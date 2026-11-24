<script lang="ts">
  import BrainIcon from '@lucide/svelte/icons/brain';

  import Markdown from '#lib/components/app/markdown/Markdown.svelte';
  import * as Trace from '#lib/components/ui/trace/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import type { PartRendererProps } from '#lib/types/shell-slots.js';

  // The model's reasoning, collapsed above the reply: a summary or the
  // reasoning text. Encrypted reasoning without a summary shows only that the
  // model reasoned.
  let { part }: PartRendererProps = $props();

  let open = $state(false);

  const text = $derived(part.content.kind === 'reasoning' ? part.content.text : '');
  const streaming = $derived(part.state === 'streaming');
  const hasText = $derived(text.trim() !== '');
</script>

{#if hasText}
  <Trace.Collapsible bind:open>
    <Trace.CollapsibleTrigger>
      <Trace.Line status={streaming ? 'running' : undefined}>
        <Trace.Icon>
          <BrainIcon />
        </Trace.Icon>
        <Trace.Title>{streaming ? m.chat_reasoning_streaming() : m.chat_reasoning_show()}</Trace.Title>
      </Trace.Line>
    </Trace.CollapsibleTrigger>
    <Trace.CollapsibleContent>
      <Markdown {streaming} {text} />
    </Trace.CollapsibleContent>
  </Trace.Collapsible>
{:else}
  <Trace.Line class="text-xs">
    <Trace.Icon>
      <BrainIcon />
    </Trace.Icon>
    <Trace.Title>{m.chat_reasoning_hidden()}</Trace.Title>
  </Trace.Line>
{/if}
