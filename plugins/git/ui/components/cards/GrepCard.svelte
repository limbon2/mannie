<script lang="ts">
  import type { ToolKindDetailsProps } from '@mannie/plugin-sdk/slots';

  import { m } from '../../generated/messages.js';
  import { matchesOf } from '../../utils/git-data.js';

  // The card of a call of git_grep (ADR-0039): each line that matched, with
  // its revision, its path and its number.
  let { call }: ToolKindDetailsProps = $props();

  const matches = $derived(matchesOf(call.data));
</script>

{#if matches.length > 0}
  <ul class="flex flex-col font-mono text-xs" aria-label={m.git_grep_matches_label()} data-git-matches="">
    {#each matches as found, index (index)}
      <li class="flex min-w-0 gap-2">
        <span class="shrink-0 text-muted-foreground">
          {found.revision === null ? '' : `${found.revision}:`}{found.path}:{found.line}
        </span>
        <span class="min-w-0 truncate whitespace-pre">{found.text}</span>
      </li>
    {/each}
  </ul>
{/if}
