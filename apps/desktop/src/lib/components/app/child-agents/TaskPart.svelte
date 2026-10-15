<script lang="ts">
  import type { TaskCallView } from '#lib/generated/bindings.js';
  import type { PartRendererProps } from '#lib/types/shell-slots.js';
  import { WAIT_FOR_TASKS } from '#lib/utils/task-parts.js';

  import TaskCallNote from './TaskCallNote.svelte';
  import TaskCard from './TaskCard.svelte';
  import TaskWaitRow from './TaskWaitRow.svelte';

  // A call of a task tool in the reply (docs/features/child-agents/
  // child-agent-activity.md): the card of the task that a start or a
  // follow-up assigned, a row for a wait, or a note for a call that assigned
  // no task. The core replaces the whole part with every change, so it shows
  // the task's latest state.
  let { part }: PartRendererProps = $props();

  const call: TaskCallView | null = $derived(part.content.kind === 'task' ? part.content.call : null);
</script>

{#if call !== null}
  {#if call.tool === WAIT_FOR_TASKS}
    <TaskWaitRow {call} />
  {:else if call.task !== null}
    <TaskCard task={call.task} />
  {:else}
    <TaskCallNote {call} />
  {/if}
{/if}
