<script lang="ts">
  import { Badge } from '@mannie/plugin-sdk/components/badge';

  import type { Commit, CommitLabel } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';
  import { formatFullDate, formatRelativeTime } from '../../utils/git-format.js';

  import RefBadge from './RefBadge.svelte';

  // The content of a row of CommitList: the commit's short hash and
  // subject, and for a full commit whether it merges, the branches and tags
  // that point at it, its author and relative date.
  interface Props {
    readonly commit: Commit | CommitLabel;
  }

  let { commit }: Props = $props();

  function isFull(shown: Commit | CommitLabel): shown is Commit {
    return 'author' in shown;
  }
  function authoredOf(shown: Commit): string | null {
    const value: unknown = shown.authored;
    if (typeof value !== 'string') {
      return null;
    }
    return value.length > 0 ? value : null;
  }
</script>

<span class="shrink-0 font-mono text-xs text-muted-foreground">{commit.short}</span>
<span class="min-w-0 truncate">{commit.subject}</span>
{#if isFull(commit)}
  {const authored = $derived(authoredOf(commit))}
  {#if commit.parents.length > 1}
    <Badge class="shrink-0" variant="outline">{m.git_merge_commit()}</Badge>
  {/if}
  {#each commit.refs as reference (reference)}
    <RefBadge {reference} />
  {/each}
  <span class="ms-auto flex shrink-0 items-center gap-2 text-xs text-muted-foreground">
    <span class="max-w-28 truncate">{commit.author}</span>
    {#if authored !== null}
      <span class="tabular-nums" title={formatFullDate(authored)}>
        {formatRelativeTime(authored)}
      </span>
    {/if}
  </span>
{/if}
