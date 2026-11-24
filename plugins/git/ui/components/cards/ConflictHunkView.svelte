<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';

  import type { ConflictHunk, Side } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';

  // A conflict of a file (plugins/git/docs/conflicts-and-history.md,
  // reading conflicts): its lines on the current side, in the base and on
  // the incoming side, tinted with semantic conflict surfaces, plus optional
  // per-hunk resolution buttons when used in the Conflicts tab.
  interface Props {
    readonly busy?: boolean;
    readonly hunk: ConflictHunk;
    readonly onChoose?: (side: Side) => void;
    readonly selected?: Side | null;
  }

  interface ConflictSide {
    readonly bg: string;
    readonly id: 'base' | 'current' | 'incoming';
    readonly lines: readonly string[];
    readonly name: string;
  }

  let { busy = false, hunk, onChoose, selected = null }: Props = $props();

  const sides: ConflictSide[] = $derived([
    { bg: 'bg-git-conflict-current', id: 'current', lines: hunk.current, name: m.git_conflict_current() },
    ...(hunk.base === null
      ? []
      : [{ bg: 'bg-muted/60', id: 'base' as const, lines: hunk.base, name: m.git_conflict_base() }]),
    { bg: 'bg-git-conflict-incoming', id: 'incoming', lines: hunk.incoming, name: m.git_conflict_incoming() },
  ]);
</script>

<div class="flex flex-col gap-1.5 rounded-md border p-2" data-git-conflict-hunk={hunk.number}>
  {#if onChoose !== undefined}
    <div class="flex flex-wrap items-center justify-between gap-1.5 border-b pb-1.5">
      <span class="text-xs font-medium text-muted-foreground">
        {m.git_conflicts_hunk_number({ number: hunk.number })}
      </span>
      <div class="flex flex-wrap items-center gap-1">
        <Button
          disabled={busy}
          size="sm"
          variant={selected === 'current' ? 'default' : 'outline'}
          onclick={() => onChoose('current')}
        >
          {m.git_conflicts_accept_current()}
        </Button>
        <Button
          disabled={busy}
          size="sm"
          variant={selected === 'incoming' ? 'default' : 'outline'}
          onclick={() => onChoose('incoming')}
        >
          {m.git_conflicts_accept_incoming()}
        </Button>
        {#if hunk.base !== null}
          <Button
            disabled={busy}
            size="sm"
            variant={selected === 'base' ? 'default' : 'outline'}
            onclick={() => onChoose('base')}
          >
            {m.git_conflicts_accept_base()}
          </Button>
        {/if}
        <Button
          disabled={busy}
          size="sm"
          variant={selected === 'both' ? 'default' : 'outline'}
          onclick={() => onChoose('both')}
        >
          {m.git_conflicts_accept_both()}
        </Button>
      </div>
    </div>
  {/if}

  {#each sides as side (side.id)}
    <section class="flex flex-col gap-0.5">
      <h5 class="text-xs font-medium text-muted-foreground">{side.name}</h5>
      {#if side.lines.length === 0}
        <p class="text-xs text-muted-foreground">{m.git_conflict_no_lines()}</p>
      {:else}
        <pre class={['overflow-x-auto rounded-xs px-2 py-1 font-mono text-xs/5', side.bg]}>{side.lines.join('')}</pre>
      {/if}
    </section>
  {/each}
</div>
