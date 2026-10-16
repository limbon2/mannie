<script lang="ts">
  import BotIcon from '@lucide/svelte/icons/bot';
  import BoxIcon from '@lucide/svelte/icons/box';
  import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';

  import type { TaskCallView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import { taskRefusalMessage } from '#lib/utils/child-agents-messages.js';
  import { isCoreTaskTool } from '#lib/utils/task-parts.js';
  import { notRunMessage } from '#lib/utils/tools-messages.js';

  // A start or a follow-up that has no task: one that the model is still
  // writing, one that never ran, such as a call beyond the reply's budget,
  // and one that the core or a plugin's executor refused, which says why. A
  // call of an executor's tool shows the icon and the words of its kind of
  // task while the plugin is on (ADR-0031, decision 7). The activity is
  // content.
  let { call }: { call: TaskCallView } = $props();

  const slots = getShellSlots();

  const starting = $derived(call.status === 'receiving' || call.status === 'pending' || call.status === 'running');
  /** Whether the call gives a task to a plugin's executor rather than to a child agent. */
  const toExecutor = $derived(!isCoreTaskTool(call.tool));
  const kind = $derived(slots.taskKinds.entries.find((entry) => entry.tool === call.tool));
  const note = $derived(explain(call));

  function explain(current: TaskCallView): string {
    if (current.refusal !== null) {
      return taskRefusalMessage(current.refusal, (id) => slots.taskKinds.find(id));
    }
    if (current.notRun !== null) {
      return notRunMessage(current.notRun);
    }
    return starting ? m.child_agents_call_starting() : m.child_agents_call_canceled();
  }
</script>

<div class="-my-1 flex min-w-0 flex-col gap-0.5 px-2.5 py-1 text-sm text-muted-foreground" data-task-call={call.status}>
  <p class="flex items-center gap-2">
    {#if starting}
      <LoaderCircleIcon class="size-4 shrink-0 animate-spin" />
    {:else if kind !== undefined}
      <kind.icon class="size-4 shrink-0" />
    {:else if toExecutor}
      <BoxIcon class="size-4 shrink-0" />
    {:else}
      <BotIcon class="size-4 shrink-0" />
    {/if}
    {#if call.activity !== null}
      <span class="min-w-0 truncate" data-content dir="auto">{call.activity}</span>
    {:else if kind !== undefined}
      <span>{kind.untitledCall()}</span>
    {:else if toExecutor}
      <span>{m.child_agents_call_untitled_task()}</span>
    {:else}
      <span>{m.child_agents_call_untitled()}</span>
    {/if}
  </p>
  <p class="ms-6">{note}</p>
</div>
