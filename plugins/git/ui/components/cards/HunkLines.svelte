<script lang="ts">
  import type { Hunk, LineKind } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';

  // The lines of a hunk of a patch (ADR-0039): each line's numbers before
  // and after the change, a marker that says in text whether the line was
  // added or removed, the same in color, and the line's text as git wrote
  // it, with a note where the file ends without a line break. Screen
  // readers hear "added" or "removed" instead of the marker.
  interface Props {
    readonly hunk: Hunk;
  }

  let { hunk }: Props = $props();

  function lineClass(kind: LineKind): string {
    switch (kind) {
      case 'added': {
        return 'bg-diff-added text-diff-added-foreground';
      }
      case 'context': {
        return '';
      }
      case 'removed': {
        return 'bg-diff-removed text-diff-removed-foreground';
      }
    }
  }

  function marker(kind: LineKind): string {
    switch (kind) {
      case 'added': {
        return '+';
      }
      case 'context': {
        return ' ';
      }
      case 'removed': {
        return '-';
      }
    }
  }

  function spoken(kind: LineKind): string | null {
    switch (kind) {
      case 'added': {
        return m.git_diff_added_line();
      }
      case 'context': {
        return null;
      }
      case 'removed': {
        return m.git_diff_removed_line();
      }
    }
  }
</script>

<div class="overflow-x-auto font-mono text-xs/5" data-git-hunk="">
  <div class="bg-muted px-2 text-muted-foreground">{hunk.header}</div>
  {#each hunk.lines as line, index (index)}
    {const words = $derived(spoken(line.kind))}
    <div class={['flex whitespace-pre', lineClass(line.kind)]} data-git-line={line.kind}>
      <span class="w-10 shrink-0 pe-2 text-end text-muted-foreground select-none" aria-hidden="true">
        {line.old ?? ''}
      </span>
      <span class="w-10 shrink-0 pe-2 text-end text-muted-foreground select-none" aria-hidden="true">
        {line.new ?? ''}
      </span>
      <span class="w-4 shrink-0 select-none" aria-hidden="true">{marker(line.kind)}</span>
      {#if words !== null}
        <span class="sr-only">{words}</span>
      {/if}
      <span class="min-w-0">{line.text}</span>
      {#if line.noNewline}
        <span class="ms-2 shrink-0 font-sans text-muted-foreground">{m.git_diff_no_newline()}</span>
      {/if}
    </div>
  {/each}
</div>
