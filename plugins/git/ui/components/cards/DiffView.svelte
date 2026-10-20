<script lang="ts">
  import * as Collapsible from '@mannie/plugin-sdk/components/collapsible';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import ChevronRightIcon from '@mannie/plugin-sdk/icons/chevron-right';

  import type { FilePatch, FileStat } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';

  import HunkLines from './HunkLines.svelte';

  // A diff (ADR-0039; plugins/git/docs/panel.md): each changed file with its
  // line counts, which opens to its patch. A file whose patch the data does
  // not hold, because the result was cut, shows its counts only.
  interface Props {
    readonly files: readonly FileStat[];
    /** Whether every file starts open, as for a single file. */
    readonly open?: boolean;
    readonly patches: readonly FilePatch[];
  }

  let { files, open = false, patches }: Props = $props();

  const unpatched = $derived(
    patches.length === 0 ? 0 : files.filter((file) => patchOf(file.path) === undefined).length,
  );

  function patchOf(path: string): FilePatch | undefined {
    return patches.find((patch) => patch.path === path);
  }

  function stat(file: FileStat): string {
    const { added, removed } = file;
    const isBinary = added === null || removed === null;
    if (isBinary) {
      return m.git_diff_binary();
    }
    return m.git_diff_stat({ added: formatCount(added), removed: formatCount(removed) });
  }
</script>

<div class="flex flex-col gap-1" data-git-diff="">
  {#each files as file (file.path)}
    {const patch = $derived(patchOf(file.path))}
    <Collapsible.Root class="group/file rounded-md border" {open}>
      <Collapsible.Trigger
        class="flex w-full items-center gap-2 px-2 py-1 text-start text-sm hover:bg-muted"
        disabled={patch === undefined}
      >
        <ChevronRightIcon
          class="size-4 shrink-0 transition-transform group-data-[state=open]/file:rotate-90"
          aria-hidden="true"
        />
        <span class="min-w-0 truncate font-mono">{file.path}</span>
        {#if file.original !== null}
          <span class="truncate text-muted-foreground">{m.git_renamed_from({ path: file.original })}</span>
        {/if}
        <span class="ms-auto shrink-0 text-muted-foreground tabular-nums">{stat(file)}</span>
      </Collapsible.Trigger>
      {#if patch !== undefined}
        <Collapsible.Content class="border-t">
          {#if patch.hunks.length === 0}
            <p class="px-2 py-1 text-sm text-muted-foreground">
              {file.added === null ? m.git_diff_binary() : m.git_diff_no_changes()}
            </p>
          {:else}
            {#each patch.hunks as hunk, index (index)}
              <HunkLines {hunk} />
            {/each}
          {/if}
        </Collapsible.Content>
      {/if}
    </Collapsible.Root>
  {/each}
  {#if unpatched > 0}
    <p class="text-sm text-muted-foreground">{m.git_diff_patches_left({ count: unpatched })}</p>
  {/if}
</div>
