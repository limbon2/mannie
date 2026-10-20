<script lang="ts">
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { formatCount } from '@mannie/plugin-sdk/format';

  import type { Ref } from '../../generated/bindings.js';
  import { m } from '../../generated/messages.js';

  // A list of branches, remote branches or tags under its heading
  // (plugins/git/docs/tools.md, git_refs): each with its short hash, its
  // subject, and for a branch whether it is the current one and how it
  // stands against its upstream.
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
</script>

{#if refs.length > 0}
  <section class="flex flex-col gap-1" data-git-refs="">
    <h4 class="text-xs font-medium text-muted-foreground">{heading}</h4>
    <ul class="flex flex-col">
      {#each refs as reference (reference.name)}
        {const standing = $derived(trackingLine(reference))}
        <li class="flex min-w-0 items-center gap-2 text-sm">
          <span class="min-w-0 truncate font-mono">{reference.name}</span>
          {#if reference.current}
            <Badge variant="secondary">{m.git_refs_current()}</Badge>
          {/if}
          <span class="shrink-0 font-mono text-muted-foreground">{reference.short}</span>
          {#if standing !== null}
            <span class="shrink-0 text-xs text-muted-foreground">{standing}</span>
          {/if}
          <span class="ms-auto min-w-0 truncate text-muted-foreground">{reference.subject}</span>
        </li>
      {/each}
    </ul>
  </section>
{/if}
