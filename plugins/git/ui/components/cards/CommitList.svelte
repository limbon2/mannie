<script lang="ts">
  import type { Commit, CommitLabel } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';

  import CommitRow from './CommitRow.svelte';

  // A list of commits, newest first (plugins/git/docs/tools.md, git_log):
  // each with its short hash, its subject, and for a full commit its
  // author and the branches and tags that point at it. A choice of a
  // commit, when the panel offers one, calls `choose`.
  interface Props {
    /** Chooses a commit, such as to show its changes. */
    readonly choose?: (oid: string) => void;
    readonly commits: readonly (Commit | CommitLabel)[];
    /** The commit that is chosen, which its row marks. */
    readonly current?: string | null;
  }

  let { choose, commits, current = null }: Props = $props();
</script>

<ol class="flex flex-col" aria-label={m.git_commits_label()} data-git-commits="">
  {#each commits as commit (commit.oid)}
    <li class="min-w-0">
      {#if choose === undefined}
        <div class="flex flex-wrap items-center gap-2 text-sm"><CommitRow {commit} /></div>
      {:else}
        <button
          class="flex w-full flex-wrap items-center gap-2 rounded-sm px-1 text-start text-sm hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none aria-current:bg-muted"
          aria-current={current === commit.oid ? 'true' : undefined}
          type="button"
          onclick={() => choose(commit.oid)}
        >
          <CommitRow {commit} />
        </button>
      {/if}
    </li>
  {/each}
</ol>
