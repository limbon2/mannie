<script lang="ts">
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
  import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as Collapsible from '#lib/components/ui/collapsible/index.js';
  import type { ToolCallView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { secondClock } from '#lib/state/second-clock.js';
  import { toolsActions } from '#lib/state/tools-actions.svelte.js';
  import type { PartRendererProps } from '#lib/types/shell-slots.js';
  import { notRunMessage, toolFailureMessage, toolsFailureMessage } from '#lib/utils/tools-messages.js';
  import { callStatus, callSummary, RUN_COMMAND, toolIcon } from '#lib/utils/tools-summaries.js';

  import ToolApproval from './ToolApproval.svelte';
  import ToolCallDetails from './ToolCallDetails.svelte';

  // A tool call in the reply (docs/features/system-tools/tool-activity.md,
  // tool calls in the reply): one row, like the row of reasoning, with the
  // activity that the model named or the line that Mannie built, and the
  // statuses that need the owner's eye. The details open below the row, and
  // a request for the owner waits below it with what the core prepared. The
  // core replaces the whole call with every change, so the row shows its
  // latest state. A status region announces a request without taking the
  // focus.
  let { conversationId, part }: PartRendererProps = $props();

  let open = $state(false);

  const call: ToolCallView | null = $derived(part.content.kind === 'tool_call' ? part.content.call : null);
  const runsCommand = $derived(
    call?.tool === RUN_COMMAND && (call.status === 'executing' || call.status === 'running'),
  );
  // Only a running command reads the clock, so other rows start no timer.
  const now = $derived(runsCommand ? secondClock.now : 0);
  const summary = $derived(call === null ? '' : callSummary(call));
  const activity = $derived(call?.activity ?? null);
  const Icon = $derived(toolIcon(call?.tool ?? ''));
  const request = $derived(call?.status === 'awaiting_approval' ? call.request : null);
  const decisionFailure = $derived(toolsActions.decisionFailures[part.id] ?? null);
  const explanation = $derived(call === null ? null : explain(call));

  /** What the owner should know about how the call ended, beyond its status. */
  function explain(current: ToolCallView): string | null {
    switch (current.status) {
      case 'canceled': {
        return m.tools_canceled_description();
      }
      case 'completed': {
        const outcome = current.result?.outcome;
        return outcome?.kind === 'failed' ? toolFailureMessage(outcome.failure) : null;
      }
      case 'denied': {
        return m.tools_denied_description();
      }
      case 'not_run': {
        return current.notRun === null ? null : notRunMessage(current.notRun);
      }
      case 'uncertain': {
        return m.tools_uncertain_description();
      }
      case 'awaiting_approval':
      case 'executing':
      case 'pending':
      case 'receiving':
      case 'running': {
        return null;
      }
    }
  }
</script>

{#if call !== null}
  {const status = $derived(callStatus(call, now))}
  <!-- The negative margins bring consecutive rows closer together than the
       reply's other parts, so a run of tool calls reads as one list. -->
  <div class="-my-1 flex min-w-0 flex-col gap-2" data-tool-status={call.status}>
    <Collapsible.Root bind:open>
      <Collapsible.Trigger>
        {#snippet child({ props })}
          <Button
            {...props}
            class="max-w-full justify-start text-muted-foreground"
            data-tool-toggle
            size="sm"
            variant="ghost"
          >
            {#if status.tone === 'busy'}
              <LoaderCircleIcon class="animate-spin" data-icon="inline-start" />
            {:else}
              <Icon class={[status.tone === 'failed' && 'text-destructive']} data-icon="inline-start" />
            {/if}
            {#if activity === null}
              <span class="truncate">{summary}</span>
            {:else}
              <span class="truncate" data-content dir="auto">{activity}</span>
            {/if}
            {#if status.quiet}
              <span class="sr-only">{status.label}</span>
            {:else}
              <span
                class={[
                  'shrink-0',
                  status.tone === 'attention' && 'font-medium text-foreground',
                  status.tone === 'failed' && 'text-destructive',
                ]}>{status.label}</span
              >
            {/if}
            <ChevronDownIcon class={['transition-transform', open && 'rotate-180']} data-icon="inline-end" />
          </Button>
        {/snippet}
      </Collapsible.Trigger>
      <Collapsible.Content class="ms-3 mt-1 flex min-w-0 flex-col gap-2 border-s-2 ps-3">
        {#if activity !== null}
          <p class="text-sm wrap-anywhere">{summary}</p>
        {/if}
        {#if explanation !== null}
          <p class={['text-sm', status.tone === 'failed' && 'text-destructive']}>{explanation}</p>
        {/if}
        <ToolCallDetails {call} {conversationId} />
      </Collapsible.Content>
    </Collapsible.Root>
    {#if decisionFailure !== null}
      <p class="text-sm text-destructive" role="alert">{toolsFailureMessage(decisionFailure)}</p>
    {/if}
    {#if request !== null}
      <ToolApproval accesses={call.accesses} {conversationId} partId={part.id} {request} />
    {/if}
  </div>
  <!-- Announces a request when the call starts to wait for the owner. -->
  <p class="sr-only" role="status">
    {#if request !== null}
      <span>{m.tools_approval_announce({ summary })}</span>
    {/if}
  </p>
{/if}
