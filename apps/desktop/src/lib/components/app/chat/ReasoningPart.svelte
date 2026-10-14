<script lang="ts">
  import BrainIcon from '@lucide/svelte/icons/brain';
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';

  import Markdown from '#lib/components/app/markdown/Markdown.svelte';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Collapsible from '#lib/components/ui/collapsible/index.js';
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
  <Collapsible.Root bind:open>
    <Collapsible.Trigger>
      {#snippet child({ props })}
        <Button {...props} class="text-muted-foreground" size="sm" variant="ghost">
          <BrainIcon data-icon="inline-start" />
          {streaming ? m.chat_reasoning_streaming() : m.chat_reasoning_show()}
          <ChevronDownIcon class={['transition-transform', open && 'rotate-180']} data-icon="inline-end" />
        </Button>
      {/snippet}
    </Collapsible.Trigger>
    <Collapsible.Content class="ms-3 border-s-2 ps-3 text-muted-foreground">
      <Markdown {streaming} {text} />
    </Collapsible.Content>
  </Collapsible.Root>
{:else}
  <p class="flex items-center gap-2 text-xs text-muted-foreground">
    <BrainIcon class="size-3.5" aria-hidden="true" />
    <span>{m.chat_reasoning_hidden()}</span>
  </p>
{/if}
