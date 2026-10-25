<script lang="ts">
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as Collapsible from '#lib/components/ui/collapsible/index.js';
  import type { ContextItemView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import type { PartRendererProps } from '#lib/types/shell-slots.js';
  import { contextIcon, contextSummary, locationText } from '#lib/utils/context-kinds.js';

  // What the conversation's attachments gave for the owner's message, below
  // it (ADR-0046, decisions 3 and 6): a row that says what was searched and
  // what it gave, which opens to the excerpts that the model read, each with
  // where it comes from. A part without excerpts says why, and opens nothing.
  // The excerpts are the stored ones, so they show while the plugin is off.
  let { part }: PartRendererProps = $props();

  const slots = getShellSlots();

  let open = $state(false);

  /** The part with the words of its kind, or `null` for a part of another kind. */
  const shown = $derived.by(() => {
    if (part.content.kind !== 'context') {
      return null;
    }
    const { context } = part.content;
    const kind = slots.contextKinds.find(context.provider);
    return {
      Icon: contextIcon(kind),
      items: context.items,
      opener: kind?.open,
      provider: context.provider,
      summary: contextSummary(kind, context),
    };
  });

  function openItem(item: ContextItemView): void {
    shown?.opener?.run(item.key);
  }
</script>

{#if shown !== null}
  {#if shown.items.length === 0}
    <p class="flex items-center gap-1.5 self-end text-xs text-muted-foreground" data-context={shown.provider}>
      <shown.Icon class="size-3.5 shrink-0" aria-hidden="true" />
      <span>{shown.summary}</span>
    </p>
  {:else}
    <Collapsible.Root class="flex flex-col items-end" data-context={shown.provider} bind:open>
      <Collapsible.Trigger>
        {#snippet child({ props })}
          <Button
            {...props}
            class="max-w-full justify-start text-muted-foreground"
            data-context-toggle
            size="sm"
            variant="ghost"
          >
            <shown.Icon data-icon="inline-start" />
            <span class="truncate">{shown.summary}</span>
            <ChevronDownIcon class={['transition-transform', open && 'rotate-180']} data-icon="inline-end" />
          </Button>
        {/snippet}
      </Collapsible.Trigger>
      <Collapsible.Content class="mt-1 w-full border-s-2 ps-3">
        <ol class="flex flex-col gap-3" aria-label={m.chat_context_items_label()}>
          {#each shown.items as item (item.number)}
            {const location = $derived(locationText(item.location))}
            <li class="flex min-w-0 flex-col gap-1 text-sm" data-context-item={item.number}>
              <p class="flex flex-wrap items-baseline gap-x-2 text-xs text-muted-foreground">
                <span class="font-mono">[{item.number}]</span>
                <span class="font-medium wrap-anywhere text-foreground" data-content>{item.title}</span>
                {#if location !== null}
                  <span class="wrap-anywhere" data-content>{location}</span>
                {/if}
              </p>
              <p class="wrap-anywhere whitespace-pre-wrap" data-content>{item.text}</p>
              {#if shown.opener !== undefined}
                <Button class="self-start px-0" size="sm" variant="link" onclick={() => openItem(item)}>
                  {shown.opener.label()}
                </Button>
              {/if}
            </li>
          {/each}
        </ol>
      </Collapsible.Content>
    </Collapsible.Root>
  {/if}
{/if}
