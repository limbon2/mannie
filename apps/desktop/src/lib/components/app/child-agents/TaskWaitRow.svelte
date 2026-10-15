<script lang="ts">
  import HourglassIcon from '@lucide/svelte/icons/hourglass';
  import LoaderCircleIcon from '@lucide/svelte/icons/loader-circle';

  import type { TaskCallView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { taskStateName } from '#lib/utils/child-agents-messages.js';

  // A wait for the reports of tasks (docs/features/child-agents/
  // child-agent-activity.md, the card of a task): one row, like a tool
  // call's, that names the tasks it waits for and whether their reports
  // came. The titles are content.
  let { call }: { call: TaskCallView } = $props();

  const waiting = $derived(call.status === 'running');
</script>

<div class="-my-1 flex min-w-0 flex-col gap-1 text-sm text-muted-foreground" data-task-wait={call.status}>
  <p class="flex items-center gap-2 px-2.5 py-1">
    {#if waiting}
      <LoaderCircleIcon class="size-4 shrink-0 animate-spin" />
      <span>{m.child_agents_wait_running()}</span>
    {:else}
      <HourglassIcon class="size-4 shrink-0" />
      <span>{call.awaited.length === 0 ? m.child_agents_wait_nothing() : m.child_agents_wait_done()}</span>
    {/if}
  </p>
  {#if call.awaited.length > 0}
    <ul class="ms-8 flex flex-col gap-0.5" aria-label={m.child_agents_wait_label()}>
      {#each call.awaited as task (task.number)}
        <li class="flex min-w-0 flex-wrap items-baseline gap-x-2">
          <span class="text-foreground">{m.child_agents_task_number({ number: String(task.number) })}</span>
          <span class="min-w-0 truncate" data-content dir="auto">{task.title}</span>
          <span>{taskStateName(task.state)}</span>
        </li>
      {/each}
    </ul>
  {/if}
</div>
