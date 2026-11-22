<script lang="ts">
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { formatCount } from '@mannie/plugin-sdk/format';

  import type { Ref } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';
  import { formatFullDate, formatRelativeTime } from '../../utils/git-format.js';

  // A list of branches, remote branches or tags under its heading
  // (plugins/git/docs/tools.md, git_refs): clean two-line rows so branch
  // names, tracking status and commit subjects never collide horizontally.
  interface Props {
    readonly heading: string;
    readonly refs: readonly Ref[];
  }

  let { heading, refs }: Props = $props();

  function trackingLine(reference: Ref): string | null {
    const { tracking, upstream } = reference;
    const isTracked = upstream !== null && tracking !== null;
    if (!isTracked) {
      return null;
    }
    if (tracking.state === 'gone') {
      return m.git_refs_gone({ upstream });
    }
    return m.git_refs_ahead_behind({
      ahead: formatCount(tracking.ahead),
      behind: formatCount(tracking.behind),
      upstream,
    });
  }
  function hasMetadata(reference: Ref): boolean {
    const hasSubject = reference.subject.length > 0;
    const hasDate = reference.date.length > 0;
    return hasSubject || hasDate;
  }
</script>

{#if refs.length > 0}
  <section class="flex flex-col gap-1" data-git-refs="">
    <h4 class="text-xs font-medium text-muted-foreground">{heading}</h4>
    <ul class="flex flex-col divide-y rounded-md border">
      {#each refs as reference (reference.name)}
        {const standing = $derived(trackingLine(reference))}
        <li class="flex min-w-0 flex-col gap-0.5 px-2.5 py-1.5 text-sm">
          <div class="flex min-w-0 items-center gap-1.5">
            <span class="min-w-0 truncate font-mono font-medium" title={reference.name}>{reference.name}</span>
            {#if reference.current}
              <Badge class="shrink-0" variant="secondary">{m.git_refs_current()}</Badge>
            {/if}
            {#if standing !== null}
              <span class="shrink-0 text-xs text-muted-foreground">{standing}</span>
            {/if}
            <span class="ms-auto shrink-0 font-mono text-xs text-muted-foreground">{reference.short}</span>
          </div>
          {#if hasMetadata(reference)}
            <div class="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
              <span class="min-w-0 truncate">{reference.subject}</span>
              {#if reference.date.length > 0}
                <span class="ms-auto shrink-0 tabular-nums" title={formatFullDate(reference.date)}>
                  {formatRelativeTime(reference.date)}
                </span>
              {/if}
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  </section>
{/if}
