<script lang="ts">
  import BotIcon from '@lucide/svelte/icons/bot';
  import BoxIcon from '@lucide/svelte/icons/box';
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';
  import CircleCheckIcon from '@lucide/svelte/icons/circle-check';
  import CircleStopIcon from '@lucide/svelte/icons/circle-stop';
  import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';
  import MessagesSquareIcon from '@lucide/svelte/icons/messages-square';
  import SquareIcon from '@lucide/svelte/icons/square';
  import { untrack } from 'svelte';

  import Markdown from '#lib/components/app/markdown/Markdown.svelte';
  import PluginsSettingsLink from '#lib/components/app/plugins/PluginsSettingsLink.svelte';
  import PartOutlet from '#lib/components/app/shell/PartOutlet.svelte';
  import { Badge } from '#lib/components/ui/badge/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Collapsible from '#lib/components/ui/collapsible/index.js';
  import type { TaskView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { childAgentsActions } from '#lib/state/child-agents-actions.svelte.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import { sidePanels } from '#lib/state/side-panels.svelte.js';
  import { chatFailureMessage, runFailureMessage } from '#lib/utils/chat-errors.js';
  import { CONVERSATION_PANEL } from '#lib/utils/chat-urls.js';
  import { agentLabel, taskStateLabel } from '#lib/utils/child-agents-messages.js';
  import { failedForPluginOff } from '#lib/utils/plugins-messages.js';
  import { approvalPart, isTaskActive, isTaskTroubled } from '#lib/utils/task-parts.js';

  // The card of a task in its parent's reply (docs/features/child-agents/
  // child-agent-activity.md, the card of a task): the child's number and
  // title, its agent, its state and what it does now, the call of the child
  // that waits for the owner, and on demand the instruction and the report.
  // A task of a plugin's executor shows its target's name in place of the
  // agent, with the icon and the words of its kind of task while the plugin
  // is on (ADR-0031, decision 7). The title, the activity, the instruction,
  // the report and the target's name are content, which a model or the
  // target wrote. A status region announces a change of state without taking
  // the focus.
  let { task }: { task: TaskView } = $props();

  const slots = getShellSlots();

  let detailsOpen = $state(false);

  const number = $derived(String(task.number));
  /** The target of a plugin's executor that runs the task, or `null` for a child agent. */
  const target = $derived(task.executor ?? null);
  const kind = $derived(target === null ? undefined : slots.taskKinds.find(target.kind));
  const active = $derived(isTaskActive(task.state));
  const troubled = $derived(isTaskTroubled(task.state));
  const stateLabel = $derived(taskStateLabel(task));
  const approval = $derived(approvalPart(task));
  const waitsForOwner = $derived(task.approval !== null);
  /** What the child does now, while it works. */
  const currentActivity = $derived(active ? task.activity : null);
  const stopping = $derived(childAgentsActions.stopping.has(task.conversationId));
  const stopFailure = $derived(childAgentsActions.stopFailures.get(task.conversationId) ?? null);
  // A card shown with the conversation announces nothing until its state changes.
  const shownState = untrack(() => stateLabel);
  const announcement = $derived(
    stateLabel === shownState ? '' : m.child_agents_announce({ number, state: stateLabel }),
  );

  function openConversation(event: MouseEvent): void {
    const opener = event.currentTarget instanceof HTMLElement ? event.currentTarget : null;
    const label = { isContent: true, text: m.child_agents_tab_label({ number, title: task.title }) };
    void sidePanels.open(CONVERSATION_PANEL, task.conversationId, { label, opener });
  }
</script>

<div class="flex flex-col gap-3 rounded-lg border p-3" data-task-number={task.number} data-task-state={task.state}>
  <div class="flex items-start gap-2">
    {#if target === null}
      <BotIcon class="mt-0.5 size-4 shrink-0 text-muted-foreground" />
    {:else if kind === undefined}
      <BoxIcon class="mt-0.5 size-4 shrink-0 text-muted-foreground" />
    {:else}
      <kind.icon class="mt-0.5 size-4 shrink-0 text-muted-foreground" />
    {/if}
    <p class="flex min-w-0 flex-1 flex-wrap items-center gap-x-2 gap-y-1 text-sm">
      <span class="font-medium">{m.child_agents_task_number({ number })}</span>
      <span class="min-w-0 wrap-anywhere" data-content dir="auto">{task.title}</span>
      {#if target === null}
        <Badge variant="secondary">{agentLabel(task.agent)}</Badge>
      {:else}
        <Badge class="max-w-full" data-task-kind={target.kind} data-task-target="" variant="secondary">
          <span class="sr-only">{kind?.targetPrefix() ?? m.child_agents_target_prefix()}</span>
          <span class="min-w-0 truncate" data-content dir="auto">{target.targetName}</span>
        </Badge>
      {/if}
      {#if task.followUp}
        <Badge variant="outline">{m.child_agents_follow_up()}</Badge>
      {/if}
    </p>
  </div>
  <div class="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-1 text-sm">
    <span class="flex items-center gap-1.5">
      {#if active}
        <LoaderCircleIcon class="size-4 animate-spin text-muted-foreground" />
      {:else if troubled}
        <CircleAlertIcon class="size-4 text-destructive" />
      {:else if task.state === 'canceled'}
        <CircleStopIcon class="size-4 text-muted-foreground" />
      {:else}
        <CircleCheckIcon class="size-4 text-muted-foreground" />
      {/if}
      <span class={[troubled && 'text-destructive', waitsForOwner && 'font-medium']}>{stateLabel}</span>
    </span>
    {#if currentActivity !== null}
      <span class="min-w-0 truncate text-muted-foreground" data-content dir="auto">{currentActivity}</span>
    {/if}
    {#if task.toolCalls > 0}
      <span class="text-muted-foreground">{m.child_agents_tool_calls({ count: task.toolCalls })}</span>
    {/if}
  </div>
  {#if task.detached}
    <p class="text-sm text-muted-foreground" data-task-detached="">
      {kind?.detached() ?? m.child_agents_detached()}
    </p>
  {/if}
  {#if approval !== null}
    <!-- The child's call renders as a part of the child's conversation, so the
         owner's decision names the child's conversation, part and digest. -->
    <div class="flex flex-col gap-2 border-s-2 ps-3">
      <PartOutlet conversationId={task.conversationId} latest part={approval} />
    </div>
  {/if}
  <Collapsible.Root bind:open={detailsOpen}>
    <div class="flex flex-wrap items-center gap-2">
      <Button data-task-open="" size="sm" variant="outline" onclick={openConversation}>
        <MessagesSquareIcon data-icon="inline-start" />
        {m.child_agents_open_conversation()}
      </Button>
      {#if active}
        <Button
          data-task-stop=""
          disabled={stopping}
          size="sm"
          variant="outline"
          onclick={() => void childAgentsActions.stop(task.conversationId)}
        >
          <SquareIcon data-icon="inline-start" />
          {m.child_agents_stop()}
        </Button>
      {/if}
      <Collapsible.Trigger>
        {#snippet child({ props })}
          <Button {...props} data-task-details="" size="sm" variant="ghost">
            {m.child_agents_details()}
            <ChevronDownIcon class={['transition-transform', detailsOpen && 'rotate-180']} data-icon="inline-end" />
          </Button>
        {/snippet}
      </Collapsible.Trigger>
    </div>
    <Collapsible.Content class="mt-3 flex flex-col gap-3 border-s-2 ps-3 text-sm">
      <div class="flex flex-col gap-1">
        <p class="font-medium">{m.child_agents_instruction()}</p>
        <p class="wrap-anywhere whitespace-pre-wrap text-muted-foreground" data-content dir="auto">
          {task.instruction}
        </p>
      </div>
      {#if task.state === 'reported'}
        <div class="flex flex-col gap-1">
          <p class="font-medium">{m.child_agents_report()}</p>
          {#if task.report === null}
            <p class="text-muted-foreground">{m.child_agents_no_report()}</p>
          {:else}
            <Markdown text={task.report} />
          {/if}
        </div>
      {:else if task.failure !== null}
        <p class="text-destructive">{m.child_agents_failed({ cause: runFailureMessage(task.failure) })}</p>
      {/if}
    </Collapsible.Content>
  </Collapsible.Root>
  {#if stopFailure !== null}
    <p class="text-sm text-destructive" role="alert">{chatFailureMessage(stopFailure)}</p>
    {#if failedForPluginOff(stopFailure)}
      <PluginsSettingsLink />
    {/if}
  {/if}
</div>
<p class="sr-only" role="status">
  {#if announcement !== ''}
    {#key announcement}
      <span>{announcement}</span>
    {/key}
  {/if}
</p>
