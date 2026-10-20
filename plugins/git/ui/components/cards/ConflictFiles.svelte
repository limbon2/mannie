<script lang="ts">
  import type { ConflictedFile } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';
  import { conflictWords } from '../../utils/git-messages.js';

  // The conflicted files of an operation that stopped (plugins/git/docs/
  // conflicts-and-history.md, reading conflicts): each with how it conflicts
  // and, for a file that both sides have, how many conflicts it has left.
  interface Props {
    readonly files: readonly ConflictedFile[];
  }

  let { files }: Props = $props();

  function left(file: ConflictedFile): string | null {
    const markers = file.markers;
    return markers?.state === 'left' ? m.git_conflict_hunks_left({ count: markers.hunks }) : null;
  }
</script>

<ul class="flex flex-col gap-1" data-git-conflicts="">
  {#each files as file (file.path)}
    {const remaining = $derived(left(file))}
    <li class="flex min-w-0 items-baseline gap-2 text-sm">
      <span class="min-w-0 truncate font-mono">{file.path}</span>
      <span class="shrink-0 text-muted-foreground">{conflictWords(file.conflict)}</span>
      {#if remaining !== null}
        <span class="ms-auto shrink-0 text-muted-foreground">{remaining}</span>
      {/if}
    </li>
  {/each}
</ul>
