<script lang="ts">
  import BellRingIcon from '@lucide/svelte/icons/bell-ring';

  import { m } from '#lib/paraglide/messages.js';
  import type { PartRendererProps } from '#lib/types/shell-slots.js';
  import { endingLabel, noteTitle } from '#lib/utils/tools-background.js';
  import { firstLine } from '#lib/utils/tools-summaries.js';

  // The request of a reply that the end of background commands started
  // (docs/features/system-tools/wake-ups.md, the part `command_note`): a
  // row in place of the owner's message, which names the commands and how
  // they ended. The model read Mannie's note of them; their texts are
  // content, shown as the model wrote them.
  let { part }: PartRendererProps = $props();

  const note = $derived(part.content.kind === 'command_note' ? part.content.note : null);
</script>

{#if note !== null}
  <div class="flex min-w-0 flex-col gap-1 text-sm text-muted-foreground" data-command-note="">
    <p class="flex items-center gap-2 font-medium">
      <BellRingIcon class="size-4 shrink-0" aria-hidden="true" />
      <span>{noteTitle(note)}</span>
    </p>
    <ul class="flex flex-col gap-0.5 ps-6">
      {#each note.ends as end (end.number)}
        <li class="min-w-0">
          <span
            >{m.tools_background_note_end({
              number: end.number,
              state: endingLabel(end.ending, end.exitCode, end.signal),
            })}</span
          >
          <span class="font-mono text-xs break-all" data-content dir="auto">{firstLine(end.command)}</span>
        </li>
      {/each}
    </ul>
  </div>
{/if}
