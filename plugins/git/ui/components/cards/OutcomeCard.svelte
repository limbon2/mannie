<script lang="ts">
  import type { ToolKindDetailsProps } from '@mannie/plugin-sdk/slots';

  import { conflictedOf, labelsOf } from '../../utils/git-data.js';
  import { textOf } from '../../utils/git-summaries.js';

  import CommitList from './CommitList.svelte';
  import ConflictFiles from './ConflictFiles.svelte';

  // The card of a merge, a rebase, a cherry-pick, a revert, a pull or a
  // continued operation (plugins/git/docs/conflicts-and-history.md): the
  // commits that it made when it completed, or the conflicted files when it
  // stopped.
  let { call }: ToolKindDetailsProps = $props();

  const outcome = $derived(textOf(call.data, 'outcome'));
</script>

{#if outcome === 'stopped'}
  <ConflictFiles files={conflictedOf(call.data)} />
{:else if outcome === 'completed'}
  {const commits = $derived(labelsOf(call.data))}
  {#if commits.length > 0}
    <CommitList {commits} />
  {/if}
{/if}
