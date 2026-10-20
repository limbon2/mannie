<script lang="ts">
  import type { ToolKindDetailsProps } from '@mannie/plugin-sdk/slots';

  import { m } from '../../generated/messages.js';
  import { movedOf, pushedOf } from '../../utils/git-data.js';
  import { textOf } from '../../utils/git-summaries.js';

  import OutcomeCard from './OutcomeCard.svelte';

  // The card of a call of git_fetch or git_push (ADR-0039): the
  // remote-tracking branches that a fetch moved, how a pull ended, or the
  // refs that a push updated with how git updated them.
  let { call, conversationId }: ToolKindDetailsProps = $props();

  const moved = $derived(movedOf(call.data));
  const pushed = $derived(pushedOf(call.data));

  function short(oid: string): string {
    return oid.slice(0, 10);
  }

  function movement(from: string | null, to: string | null): string {
    if (to === null) {
      return m.git_transfer_removed();
    }
    if (from === null) {
      return m.git_transfer_new({ commit: short(to) });
    }
    return m.git_transfer_moved({ from: short(from), to: short(to) });
  }
</script>

{#if textOf(call.data, 'outcome') !== null}
  <OutcomeCard {call} {conversationId} />
{:else if pushed.length > 0}
  <ul class="flex flex-col font-mono text-xs" data-git-pushed="">
    {#each pushed as update (update.destination)}
      <li class="flex min-w-0 gap-2">
        <span class="shrink-0">{update.destination}</span>
        <span class="min-w-0 truncate text-muted-foreground">{update.summary}</span>
      </li>
    {/each}
  </ul>
{:else if call.data !== null}
  <section class="flex flex-col gap-1" data-git-moved="">
    <h4 class="text-xs font-medium text-muted-foreground">{m.git_transfer_label()}</h4>
    {#if moved.length === 0}
      <p class="text-sm text-muted-foreground">{m.git_transfer_unchanged()}</p>
    {:else}
      <ul class="flex flex-col font-mono text-xs">
        {#each moved as change (change.name)}
          <li class="flex min-w-0 gap-2">
            <span class="shrink-0">{change.name}</span>
            <span class="min-w-0 truncate text-muted-foreground">{movement(change.from, change.to)}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/if}
