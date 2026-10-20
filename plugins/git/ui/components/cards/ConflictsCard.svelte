<script lang="ts">
  import type { ToolKindDetailsProps } from '@mannie/plugin-sdk/slots';

  import { conflictedOf, namedConflictOf } from '../../utils/git-data.js';

  import ConflictFiles from './ConflictFiles.svelte';
  import ConflictHunkView from './ConflictHunkView.svelte';

  // The card of a call of git_conflicts (ADR-0039): the conflicted files of
  // the operation in progress, and the hunks of the file that the call
  // named, each with its current, base and incoming lines.
  let { call }: ToolKindDetailsProps = $props();

  const files = $derived(conflictedOf(call.data));
  const named = $derived(namedConflictOf(call.data));
</script>

{#if files.length > 0}
  <ConflictFiles {files} />
{/if}
{#if named !== null}
  <div class="flex flex-col gap-2" data-git-named-conflict="">
    <p class="truncate font-mono text-xs text-muted-foreground">{named.path}</p>
    {#each named.hunks as hunk (hunk.number)}
      <ConflictHunkView {hunk} />
    {/each}
  </div>
{/if}
