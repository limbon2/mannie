<script lang="ts">
  import type { ToolKindDetailsProps } from '@mannie/plugin-sdk/slots';

  import { m } from '../../generated/messages.js';
  import { blameOf } from '../../utils/git-data.js';

  // Transcript card for `git_blame` tool calls: grouped commit runs with
  // short hash, author, day, summary and numbered code lines.
  let { call }: ToolKindDetailsProps = $props();

  const blame = $derived(blameOf(call.data));
  const runs = $derived(blame?.runs ?? []);
  const heading = $derived.by(() => {
    if (blame === null) {
      return null;
    }
    if (blame.revision === null) {
      return m.git_blame_card_title({ path: blame.path });
    }
    return m.git_blame_card_at({ path: blame.path, revision: blame.revision });
  });
</script>

{#if runs.length > 0}
  <div class="flex flex-col gap-1.5 rounded-md border" data-git-blame-card="">
    {#if heading !== null}
      <div class="border-b bg-muted/40 px-2 py-1 font-mono text-xs text-muted-foreground">
        {heading}
      </div>
    {/if}

    <div class="max-h-72 overflow-auto font-mono text-xs/5">
      {#each runs as run, runIndex (`${run.commit.oid}-${runIndex}`)}
        <div class="border-b last:border-b-0">
          <div class="flex items-center gap-2 bg-muted/30 px-2 py-0.5 font-sans text-xs text-muted-foreground">
            {#if run.commit.uncommitted}
              <span class="font-medium text-git-untracked">{m.git_inspect_blame_uncommitted()}</span>
            {:else}
              <span class="shrink-0 font-mono text-foreground">{run.short}</span>
              <span class="shrink-0">{run.commit.author}</span>
              {#if run.commit.day.length > 0}
                <span class="shrink-0 tabular-nums">{run.commit.day}</span>
              {/if}
              <span class="min-w-0 truncate">{run.commit.summary}</span>
            {/if}
          </div>

          {#each run.lines as line (line.number)}
            <div class="flex px-2 whitespace-pre">
              <span class="w-10 shrink-0 pe-2 text-end text-muted-foreground select-none" aria-hidden="true">
                {line.number}
              </span>
              <span class="min-w-0">{line.text}</span>
            </div>
          {/each}
        </div>
      {/each}
    </div>
  </div>
{/if}
