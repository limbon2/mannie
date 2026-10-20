<script lang="ts">
  import type { ConflictHunk } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';

  // A conflict of a file (plugins/git/docs/conflicts-and-history.md,
  // reading conflicts): its lines on the current side, in the base and on
  // the incoming side, each under its name, as git_conflicts shows them.
  interface Props {
    readonly hunk: ConflictHunk;
  }

  interface Side {
    readonly id: string;
    readonly lines: readonly string[];
    readonly name: string;
  }

  let { hunk }: Props = $props();

  const sides: Side[] = $derived([
    { id: 'current', lines: hunk.current, name: m.git_conflict_current() },
    ...(hunk.base === null ? [] : [{ id: 'base', lines: hunk.base, name: m.git_conflict_base() }]),
    { id: 'incoming', lines: hunk.incoming, name: m.git_conflict_incoming() },
  ]);
</script>

<div class="flex flex-col gap-1 rounded-md border p-2" data-git-conflict-hunk={hunk.number}>
  {#each sides as side (side.id)}
    <section class="flex flex-col">
      <h5 class="text-xs font-medium text-muted-foreground">{side.name}</h5>
      {#if side.lines.length === 0}
        <p class="text-xs text-muted-foreground">{m.git_conflict_no_lines()}</p>
      {:else}
        <pre class="overflow-x-auto bg-muted px-2 font-mono text-xs/5">{side.lines.join('')}</pre>
      {/if}
    </section>
  {/each}
</div>
