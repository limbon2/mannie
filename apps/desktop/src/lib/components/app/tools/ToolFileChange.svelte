<script lang="ts">
  import type { FileChangeLineKind } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { formatCount } from '#lib/utils/chat-errors.js';
  import type { SavedFileChange } from '#lib/utils/tools-file-changes.js';

  let { file }: { file: SavedFileChange } = $props();
  const id = $props.id();
  const unchanged = $derived(file.change.added === 0 && file.change.removed === 0);

  function lineClass(kind: FileChangeLineKind): string {
    switch (kind) {
      case 'added':
        return 'bg-diff-added text-diff-added-foreground';
      case 'removed':
        return 'bg-diff-removed text-diff-removed-foreground';
      case 'context':
        return '';
    }
  }

  function marker(kind: FileChangeLineKind): string {
    switch (kind) {
      case 'added':
        return m.tools_file_change_added_marker();
      case 'removed':
        return m.tools_file_change_removed_marker();
      case 'context':
        return ' ';
    }
  }
</script>

<div class="flex min-w-0 flex-col gap-2 text-sm" data-file-change>
  <p {id} class="wrap-anywhere" data-content dir="auto">{file.path}</p>
  {#if unchanged}
    <p class="text-muted-foreground">{m.tools_file_change_empty()}</p>
  {:else if file.change.hunks.length > 0}
    <!-- Focusable for keyboard scrolling (WCAG 2.1.1). Positioned so each line's
         absolute screen-reader label stays inside this scroll area instead of
         extending the transcript beyond the visible diff. -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <div
      class="relative max-h-96 overflow-auto rounded-md border font-mono text-xs/5 outline-none focus-visible:ring-2 focus-visible:ring-ring"
      aria-labelledby={id}
      dir="ltr"
      role="region"
      tabindex="0"
    >
      <div class="w-max min-w-full" data-content>
        {#each file.change.hunks as hunk, index (index)}
          <div class="bg-muted px-2 text-muted-foreground">
            {m.tools_file_change_hunk({
              newLines: formatCount(hunk.newLines),
              newStart: formatCount(hunk.newStart),
              oldLines: formatCount(hunk.oldLines),
              oldStart: formatCount(hunk.oldStart),
            })}
          </div>
          {#each hunk.lines as line, lineIndex (lineIndex)}
            <div class={['flex whitespace-pre', lineClass(line.kind)]} data-file-line={line.kind}>
              <span class="w-12 shrink-0 pe-2 text-end select-none" aria-hidden="true"
                >{line.old === null ? '' : formatCount(line.old)}</span
              >
              <span class="w-12 shrink-0 pe-2 text-end select-none" aria-hidden="true"
                >{line.new === null ? '' : formatCount(line.new)}</span
              >
              <span class="w-4 shrink-0 select-none" aria-hidden="true">{marker(line.kind)}</span>
              <span class="sr-only">
                {#if line.kind === 'added'}{m.tools_file_change_added_line()}{:else if line.kind === 'removed'}{m.tools_file_change_removed_line()}{/if}
                {#if line.old !== null}{m.tools_file_change_old_line({ number: formatCount(line.old) })}{/if}
                {#if line.new !== null}{m.tools_file_change_new_line({ number: formatCount(line.new) })}{/if}
              </span>
              <span>{line.text}</span>
              {#if line.textTruncated}<span class="ms-2 font-sans">{m.tools_file_change_line_cut()}</span>{/if}
              {#if line.noNewline}<span class="ms-2 font-sans">{m.tools_file_change_no_newline()}</span>{/if}
            </div>
          {/each}
        {/each}
      </div>
    </div>
  {/if}
  {#if file.change.truncated}
    <p class="text-xs text-muted-foreground">{m.tools_file_change_truncated()}</p>
  {/if}
</div>
