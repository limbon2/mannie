<script lang="ts">
  import { Meter } from 'bits-ui';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import type { CheckpointView, ContextUsage } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatActions } from '#lib/state/chat-actions.svelte.js';
  import { formatCount } from '#lib/utils/chat-errors.js';

  // The share of the context budget that a conversation uses (docs/features/
  // chat/conversation-view.md, layout), with the exact counts, whether they
  // are estimated, the latest checkpoint and "Compact now".
  interface Props {
    checkpoint: CheckpointView | null;
    context: ContextUsage | null;
    conversationId: string;
  }

  let { checkpoint, context, conversationId }: Props = $props();

  const used = $derived(context?.usedTokens ?? 0);
  const budget = $derived(context?.budgetTokens ?? 0);
  const percent = $derived(budget === 0 ? 0 : Math.min(100, Math.round((used / budget) * 100)));
  const compacting = $derived(chatActions.compacting.includes(conversationId));
  const label = $derived(
    context === null ? m.chat_context_unknown() : m.chat_context_label({ percent: formatCount(percent) }),
  );

  function showCheckpoint(): void {
    if (checkpoint !== null) {
      void chatActions.showCheckpoint({ checkpointId: checkpoint.id, conversationId });
    }
  }
</script>

<DropdownMenu.Root>
  <DropdownMenu.Trigger>
    {#snippet child({ props })}
      <Button {...props} aria-label={label} size="sm" variant="ghost">
        <Meter.Root
          class="h-1.5 w-16 overflow-hidden rounded-full bg-muted"
          aria-hidden="true"
          max={Math.max(budget, 1)}
          value={used}
        >
          <div class="h-full rounded-full bg-primary" style:width="{percent}%"></div>
        </Meter.Root>
        <span class="text-xs text-muted-foreground tabular-nums">{label}</span>
      </Button>
    {/snippet}
  </DropdownMenu.Trigger>
  <DropdownMenu.Content class="w-72" align="end">
    <DropdownMenu.Group>
      <DropdownMenu.Label>{m.chat_context_title()}</DropdownMenu.Label>
      <p class="px-2 py-1 text-sm">
        {#if context === null}
          {m.chat_context_unknown_details()}
        {:else}
          {m.chat_context_details({ budget: formatCount(budget), used: formatCount(used) })}
        {/if}
      </p>
      {#if context?.estimated}
        <p class="px-2 pb-1 text-xs text-muted-foreground">{m.chat_context_estimated()}</p>
      {/if}
    </DropdownMenu.Group>
    <DropdownMenu.Separator />
    <DropdownMenu.Group>
      {#if checkpoint !== null}
        <DropdownMenu.Item onSelect={showCheckpoint}>{m.chat_checkpoint_show()}</DropdownMenu.Item>
      {/if}
      <DropdownMenu.Item disabled={compacting} onSelect={() => void chatActions.compact(conversationId)}>
        {m.chat_compact_now()}
      </DropdownMenu.Item>
    </DropdownMenu.Group>
  </DropdownMenu.Content>
</DropdownMenu.Root>
