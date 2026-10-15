<script lang="ts">
  import BotIcon from '@lucide/svelte/icons/bot';
  import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';

  import type { TaskCallView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { taskRefusalMessage } from '#lib/utils/child-agents-messages.js';
  import { notRunMessage } from '#lib/utils/tools-messages.js';

  // A start or a follow-up that has no task: one that the model is still
  // writing, one that never ran, such as a call beyond the reply's budget,
  // and one that the core refused, which says why. The activity is content.
  let { call }: { call: TaskCallView } = $props();

  const starting = $derived(call.status === 'receiving' || call.status === 'pending' || call.status === 'running');
  const note = $derived(explain(call));

  function explain(current: TaskCallView): string {
    if (current.refusal !== null) {
      return taskRefusalMessage(current.refusal);
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
    {:else}
      <BotIcon class="size-4 shrink-0" />
    {/if}
    {#if call.activity !== null}
      <span class="min-w-0 truncate" data-content dir="auto">{call.activity}</span>
    {:else}
      <span>{m.child_agents_call_untitled()}</span>
    {/if}
  </p>
  <p class="ms-6">{note}</p>
</div>
