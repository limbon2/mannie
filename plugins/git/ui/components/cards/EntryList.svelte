<script lang="ts">
  import type { Entry } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';
  import { changeWords, conflictWords } from '../../utils/git-messages.js';

  // The files of a status (plugins/git/docs/tools.md, git_status): the
  // conflicted, staged, unstaged and untracked files, each group under its
  // heading, as git_status lists them for the model. A choice of a file,
  // when the panel offers one, calls `choose` with whether it is staged.
  interface Props {
    /** Chooses a file, such as to show its diff. */
    readonly choose?: (entry: Entry, staged: boolean) => void;
    readonly entries: readonly Entry[];
    /** The files in all, of which `entries` may hold the first only. */
    readonly total: number;
  }

  interface Line {
    readonly entry: Entry;
    readonly text: string;
  }

  interface Group {
    readonly heading: string;
    readonly id: string;
    readonly lines: readonly Line[];
    readonly staged: boolean;
  }

  let { choose, entries, total }: Props = $props();

  const groups: Group[] = $derived(
    [
      {
        heading: m.git_status_conflicted(),
        id: 'conflicted',
        lines: entries.flatMap(conflictedLines),
        staged: false,
      },
      {
        heading: m.git_status_staged(),
        id: 'staged',
        lines: entries.flatMap((entry) => changeLines(entry, entry.staged)),
        staged: true,
      },
      {
        heading: m.git_status_unstaged(),
        id: 'unstaged',
        lines: entries.flatMap((entry) => changeLines(entry, entry.unstaged)),
        staged: false,
      },
      {
        heading: m.git_status_untracked(),
        id: 'untracked',
        lines: entries.flatMap(untrackedLines),
        staged: false,
      },
    ].filter((group) => group.lines.length > 0),
  );

  /** The line of a conflicted file, or none for another file. */
  function conflictedLines(entry: Entry): Line[] {
    if (entry.kind !== 'conflicted') {
      return [];
    }
    return [{ entry, text: m.git_entry_line({ change: conflictWords(entry.conflict), path: entry.path }) }];
  }

  /**
   * The line of a tracked file's change that `letter` names, in the index
   * or the work tree, or none where it has no change there.
   */
  function changeLines(entry: Entry, letter: string): Line[] {
    if (letter === '.') {
      return [];
    }
    const change = changeWords(letter);
    switch (entry.kind) {
      case 'changed': {
        return [{ entry, text: m.git_entry_line({ change, path: entry.path }) }];
      }
      case 'renamed': {
        return [{ entry, text: m.git_entry_renamed_line({ change, original: entry.original, path: entry.path }) }];
      }
      case 'conflicted':
      case 'ignored':
      case 'untracked': {
        return [];
      }
    }
  }

  /** The line of an untracked file, or none for another file. */
  function untrackedLines(entry: Entry): Line[] {
    return entry.kind === 'untracked' ? [{ entry, text: entry.path }] : [];
  }
</script>

{#if groups.length === 0}
  <p class="text-sm text-muted-foreground">{m.git_status_clean()}</p>
{:else}
  <div class="flex flex-col gap-3" data-git-entries="">
    {#each groups as group (group.id)}
      <section class="flex flex-col gap-1" aria-labelledby="git-group-{group.id}">
        <h4 id="git-group-{group.id}" class="text-xs font-medium text-muted-foreground">{group.heading}</h4>
        <ul class="flex flex-col">
          {#each group.lines as line (line.entry.path)}
            <li class="min-w-0 font-mono text-sm">
              {#if choose === undefined}
                <span class="block truncate">{line.text}</span>
              {:else}
                <button
                  class="block w-full truncate rounded-sm px-1 text-start hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
                  type="button"
                  onclick={() => choose(line.entry, group.staged)}
                >
                  {line.text}
                </button>
              {/if}
            </li>
          {/each}
        </ul>
      </section>
    {/each}
    {#if total > entries.length}
      <p class="text-sm text-muted-foreground">{m.git_status_more({ count: total - entries.length })}</p>
    {/if}
  </div>
{/if}
