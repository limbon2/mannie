<script lang="ts">
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
  import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';

  import PluginsSettingsLink from '#lib/components/app/plugins/PluginsSettingsLink.svelte';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Collapsible from '#lib/components/ui/collapsible/index.js';
  import type { ToolCallView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { getConversationContext } from '#lib/state/conversation-context.js';
  import { secondClock } from '#lib/state/second-clock.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import { toolsActions } from '#lib/state/tools-actions.svelte.js';
  import type { PartRendererProps } from '#lib/types/shell-slots.js';
  import { failedForPluginOff } from '#lib/utils/plugins-messages.js';
  import {
    callToolKind,
    findToolKind,
    pluginFailure,
    pluginFailureMessage,
    toolKindCall,
  } from '#lib/utils/tool-kinds.js';
  import { commandState, isRunning, startedCommand } from '#lib/utils/tools-background.js';
  import {
    notRunMessage,
    toolFailureMessage,
    toolsFailureMessage,
    uncertainMessage,
  } from '#lib/utils/tools-messages.js';
  import { type CallStatus, callStatus, callSummary, RUN_COMMAND, toolIcon } from '#lib/utils/tools-summaries.js';

  import BackgroundCommandActions from './BackgroundCommandActions.svelte';
  import ToolApproval from './ToolApproval.svelte';
  import ToolCallDetails from './ToolCallDetails.svelte';

  // A tool call in the reply (docs/features/system-tools/tool-activity.md,
  // tool calls in the reply): one row, like the row of reasoning, with the
  // activity that the model named or the line that Mannie built, and the
  // statuses that need the owner's eye. The details open below the row, and
  // a request for the owner waits below it with what the core prepared. The
  // core replaces the whole call with every change, so the row shows its
  // latest state. A status region announces a request without taking the
  // focus. A call of a plugin's tool takes its icon, its line, the sentence
  // of its failure and its details from the tool kind that its plugin
  // registered, if one did (ADR-0039). A call that started a command in the
  // background shows the command's state instead, with its output and Stop
  // in its details (ADR-0041).
  let { conversationId, part }: PartRendererProps = $props();

  const slots = getShellSlots();
  const context = getConversationContext();

  let open = $state(false);

  const call: ToolCallView | null = $derived(part.content.kind === 'tool_call' ? part.content.call : null);
  const kind = $derived(
    call === null ? undefined : callToolKind(call, (tool, plugin) => findToolKind(slots.toolKinds, tool, plugin)),
  );
  const runsCommand = $derived(
    call?.tool === RUN_COMMAND && (call.status === 'executing' || call.status === 'running'),
  );
  /**
   * The command that the call started in the background, while the
   * conversation shows it: a call of `start_command`, or of a plugin's tool
   * that starts one on a remote host (ADR-0041, decision 11).
   */
  const started = $derived(call === null ? undefined : startedCommand(context.backgroundCommands, part.id));
  const backgroundRuns = $derived(started !== undefined && isRunning(started));
  // Only a running command reads the clock, so other rows start no timer.
  const now = $derived(runsCommand || backgroundRuns ? secondClock.now : 0);
  const summary = $derived(call === null ? '' : lineOf(call));
  const activity = $derived(call?.activity ?? null);
  const Icon = $derived(kind?.icon ?? toolIcon(call?.tool ?? ''));
  const request = $derived(call?.status === 'awaiting_approval' ? call.request : null);
  const decisionFailure = $derived(toolsActions.decisionFailures[part.id] ?? null);
  const explanation = $derived(call === null ? null : explain(call));

  /** The row's status: the call's, or the state of the command that it started. */
  function statusOf(current: ToolCallView, at: number): CallStatus {
    if (started === undefined) {
      return callStatus(current, at);
    }
    return { label: commandState(started, at), quiet: false, tone: 'done' };
  }

  /** The call's line: its tool kind's, or the one that Mannie builds. */
  function lineOf(current: ToolCallView): string {
    const kindLine = kind === undefined ? null : kind.summary(toolKindCall(current));
    return kindLine ?? callSummary(current);
  }

  /** What the owner should know about how the call ended, beyond its status. */
  function explain(current: ToolCallView): string | null {
    switch (current.status) {
      case 'canceled': {
        return m.tools_canceled_description();
      }
      case 'completed': {
        const outcome = current.result?.outcome;
        if (outcome?.kind === 'failed') {
          return toolFailureMessage(outcome.failure);
        }
        const code = pluginFailure(current);
        return code === null ? null : pluginFailureMessage(kind, code);
      }
      case 'denied': {
        return m.tools_denied_description();
      }
      case 'not_run': {
        return current.notRun === null ? null : notRunMessage(current.notRun);
      }
      case 'uncertain': {
        return uncertainMessage(current.interruption ?? null);
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
  {const status = $derived(statusOf(call, now))}
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
        {#if started !== undefined}
          <BackgroundCommandActions command={started} {conversationId} />
        {/if}
        <ToolCallDetails {call} {conversationId} {kind} partId={part.id} />
      </Collapsible.Content>
    </Collapsible.Root>
    {#if decisionFailure !== null}
      <p class="text-sm text-destructive" role="alert">{toolsFailureMessage(decisionFailure)}</p>
      {#if failedForPluginOff(decisionFailure)}
        <PluginsSettingsLink />
      {/if}
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
