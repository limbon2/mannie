<script lang="ts">
  import PlayIcon from '@lucide/svelte/icons/play';

  import { Button } from '#lib/components/ui/button/index.js';
  import type { PlanSummary } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { conversationList } from '#lib/state/conversation-list.svelte.js';
  import { plans } from '#lib/state/plans.svelte.js';
  import type { ChatFailure } from '#lib/types/chat.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';

  // The two ways to run a plan (ADR-0060, decision 7): both fill a message box,
  // and the owner presses Send. The second shows while the plan's conversation
  // exists, and is off with a reason while a reply runs there.
  let { plan }: { plan: PlanSummary } = $props();

  let failure = $state<ChatFailure | null>(null);

  const conversation = $derived(plan.conversationId === null ? undefined : conversationList.find(plan.conversationId));
  const canUseConversation = $derived(plan.conversationId !== null);
  const replying = $derived(conversation?.hasActiveRun ?? false);

  async function runInItsConversation(): Promise<void> {
    failure = (await plans.runInItsConversation(plan)) as ChatFailure | null;
  }
</script>

<div class="flex flex-wrap items-center gap-2">
  <Button data-plan-run-new="" size="sm" variant="outline" onclick={() => void plans.runInNewConversation(plan)}>
    <PlayIcon data-icon="inline-start" />
    {m.plans_run_new()}
  </Button>
  {#if canUseConversation}
    <Button
      data-plan-run-here=""
      disabled={replying}
      size="sm"
      variant="outline"
      onclick={() => void runInItsConversation()}
    >
      {m.plans_run_here()}
    </Button>
    {#if replying}
      <span class="text-xs text-muted-foreground">{m.plans_run_busy()}</span>
    {/if}
  {/if}
  {#if failure !== null}
    <span class="text-xs text-destructive" role="alert">{chatFailureMessage(failure)}</span>
  {/if}
</div>
