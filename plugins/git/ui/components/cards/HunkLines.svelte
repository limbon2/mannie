<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import MinusIcon from '@mannie/plugin-sdk/icons/minus';
  import PlusIcon from '@mannie/plugin-sdk/icons/plus';
  import Undo2Icon from '@mannie/plugin-sdk/icons/undo-2';

  import type { Hunk, LineKind } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';
  import { annotateHunkLines, buildSplitRows } from '../../utils/git-word-diff.js';

  // The lines of a hunk of a patch (ADR-0039): supports unified and
  // side-by-side split views with word-level intra-line highlights and
  // optional per-hunk stage/unstage/discard controls. Screen readers hear
  // "added" or "removed" instead of the visual marker.
  interface Props {
    readonly busy?: boolean;
    readonly hunk: Hunk;
    readonly mode?: 'split' | 'unified';
    readonly onDiscard?: () => void;
    readonly onStage?: () => void;
    readonly onUnstage?: () => void;
  }

  let { busy = false, hunk, mode = 'unified', onDiscard, onStage, onUnstage }: Props = $props();

  const annotated = $derived(mode === 'unified' ? annotateHunkLines(hunk.lines) : []);
  const splitRows = $derived(mode === 'split' ? buildSplitRows(hunk.lines) : []);
  const hasActions = $derived(onStage !== undefined || onUnstage !== undefined || onDiscard !== undefined);

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

  function emphasisClass(kind: LineKind): string {
    if (kind === 'added') {
      return 'rounded-xs bg-diff-added-emphasis';
    }
    if (kind === 'removed') {
      return 'rounded-xs bg-diff-removed-emphasis';
    }
    return '';
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
  <div class="flex items-center justify-between gap-2 bg-muted px-2 py-0.5 text-muted-foreground">
    <span class="min-w-0 truncate">{hunk.header}</span>
    {#if hasActions}
      <div class="flex shrink-0 items-center gap-1 font-sans">
        {#if onStage !== undefined}
          <Button class="h-5 px-1.5 text-xs" disabled={busy} size="sm" variant="ghost" onclick={onStage}>
            <PlusIcon class="size-3" aria-hidden="true" />
            <span>{m.git_diff_stage_hunk()}</span>
          </Button>
        {/if}

        {#if onUnstage !== undefined}
          <Button class="h-5 px-1.5 text-xs" disabled={busy} size="sm" variant="ghost" onclick={onUnstage}>
            <MinusIcon class="size-3" aria-hidden="true" />
            <span>{m.git_diff_unstage_hunk()}</span>
          </Button>
        {/if}

        {#if onDiscard !== undefined}
          <Button
            class="h-5 px-1.5 text-xs text-destructive hover:text-destructive"
            disabled={busy}
            size="sm"
            variant="ghost"
            onclick={onDiscard}
          >
            <Undo2Icon class="size-3" aria-hidden="true" />
            <span>{m.git_diff_discard_hunk()}</span>
          </Button>
        {/if}
      </div>
    {/if}
  </div>

  {#if mode === 'split'}
    <div class="grid min-w-160 grid-cols-2 divide-x">
      {#each splitRows as row, rowIndex (rowIndex)}
        <div
          class={['flex min-w-0 whitespace-pre', row.left === null ? 'bg-muted/20' : lineClass(row.left.line.kind)]}
          data-git-line={row.left?.line.kind}
        >
          <span class="w-9 shrink-0 pe-2 text-end text-muted-foreground select-none" aria-hidden="true">
            {row.left?.line.old ?? ''}
          </span>
          <span class="w-4 shrink-0 select-none" aria-hidden="true">
            {row.left === null ? '' : marker(row.left.line.kind)}
          </span>
          {#if row.left !== null}
            {const words = $derived(spoken(row.left.line.kind))}
            {#if words !== null}
              <span class="sr-only">{words}</span>
            {/if}
            <span class="min-w-0">
              {#each row.left.segments as segment, segIndex (segIndex)}
                {#if segment.emphasized}
                  <span class={emphasisClass(row.left.line.kind)}>{segment.text}</span>
                {:else}
                  {segment.text}
                {/if}
              {/each}
            </span>
          {/if}
        </div>

        <div
          class={['flex min-w-0 whitespace-pre', row.right === null ? 'bg-muted/20' : lineClass(row.right.line.kind)]}
          data-git-line={row.right?.line.kind}
        >
          <span class="w-9 shrink-0 pe-2 text-end text-muted-foreground select-none" aria-hidden="true">
            {row.right?.line.new ?? ''}
          </span>
          <span class="w-4 shrink-0 select-none" aria-hidden="true">
            {row.right === null ? '' : marker(row.right.line.kind)}
          </span>
          {#if row.right !== null}
            {const words = $derived(spoken(row.right.line.kind))}
            {#if words !== null}
              <span class="sr-only">{words}</span>
            {/if}
            <span class="min-w-0">
              {#each row.right.segments as segment, segIndex (segIndex)}
                {#if segment.emphasized}
                  <span class={emphasisClass(row.right.line.kind)}>{segment.text}</span>
                {:else}
                  {segment.text}
                {/if}
              {/each}
            </span>
          {/if}
        </div>
      {/each}
    </div>
  {:else}
    {#each annotated as item, index (index)}
      {const words = $derived(spoken(item.line.kind))}
      <div class={['flex whitespace-pre', lineClass(item.line.kind)]} data-git-line={item.line.kind}>
        <span class="w-10 shrink-0 pe-2 text-end text-muted-foreground select-none" aria-hidden="true">
          {item.line.old ?? ''}
        </span>
        <span class="w-10 shrink-0 pe-2 text-end text-muted-foreground select-none" aria-hidden="true">
          {item.line.new ?? ''}
        </span>
        <span class="w-4 shrink-0 select-none" aria-hidden="true">{marker(item.line.kind)}</span>
        {#if words !== null}
          <span class="sr-only">{words}</span>
        {/if}
        <span class="min-w-0">
          {#each item.segments as segment, segIndex (segIndex)}
            {#if segment.emphasized}
              <span class={emphasisClass(item.line.kind)}>{segment.text}</span>
            {:else}
              {segment.text}
            {/if}
          {/each}
        </span>
        {#if item.line.noNewline}
          <span class="ms-2 shrink-0 font-sans text-muted-foreground">{m.git_diff_no_newline()}</span>
        {/if}
      </div>
    {/each}
  {/if}
</div>
