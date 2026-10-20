<script lang="ts">
  import { Badge } from '@mannie/plugin-sdk/components/badge';

  import type { Commit, CommitLabel } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';

  // The content of a row of CommitList: the commit's short hash and
  // subject, and for a full commit whether it merges, the branches and tags
  // that point at it, and its author.
  interface Props {
    readonly commit: Commit | CommitLabel;
  }

  let { commit }: Props = $props();

  function isFull(shown: Commit | CommitLabel): shown is Commit {
    return 'author' in shown;
  }
</script>

<span class="shrink-0 font-mono text-muted-foreground">{commit.short}</span>
<span class="min-w-0 truncate">{commit.subject}</span>
{#if isFull(commit)}
  {#if commit.parents.length > 1}
    <Badge variant="outline">{m.git_merge_commit()}</Badge>
  {/if}
  {#each commit.refs as reference (reference)}
    <Badge class="max-w-40 truncate" variant="secondary">{reference}</Badge>
  {/each}
  <span class="ms-auto shrink-0 text-xs text-muted-foreground">{commit.author}</span>
{/if}
