<script lang="ts">
  import ClipboardListIcon from '@lucide/svelte/icons/clipboard-list';
  import { onMount } from 'svelte';

  import { Button } from '#lib/components/ui/button/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { plans } from '#lib/state/plans.svelte.js';
  import { sidePanels } from '#lib/state/side-panels.svelte.js';
  import { PLAN_PANEL } from '#lib/utils/plans-messages.js';

  import PlanRunButtons from './PlanRunButtons.svelte';

  // The card below a call of `save_plan` (docs/features/plans.md): the plan's
  // title, Open and the two ways to run it. A conversation holds one plan, so
  // the card acts on the conversation's plan as it stands.
  interface Props {
    conversationId: string;
    /** The title that the call saved. */
    title: string;
  }

  let { conversationId, title }: Props = $props();

  let trigger = $state<HTMLElement | null>(null);

  const plan = $derived(plans.ofConversation(conversationId));

  onMount(() => {
    // The list follows every plan that the transcript shows.
    void plans.reload();
  });
</script>

<div class="ms-3 flex flex-col gap-2 rounded-lg border p-3" data-plan-card="">
  <div class="flex items-center gap-2">
    <ClipboardListIcon class="size-4 shrink-0 text-muted-foreground" aria-hidden="true" />
    <p class="min-w-0 flex-1 truncate text-sm font-medium" data-content="" dir="auto">{plan?.title ?? title}</p>
  </div>
  {#if plan === undefined}
    <p class="text-xs text-muted-foreground">{m.plans_card_gone()}</p>
  {:else}
    <div class="flex flex-wrap items-center gap-2">
      <Button
        data-plan-open=""
        size="sm"
        variant="outline"
        bind:ref={trigger}
        onclick={() =>
          void sidePanels.open(PLAN_PANEL, String(plan.number), {
            label: { isContent: true, text: plan.title },
            opener: trigger,
          })}
      >
        {m.plans_open()}
      </Button>
      <PlanRunButtons {plan} />
    </div>
  {/if}
</div>
