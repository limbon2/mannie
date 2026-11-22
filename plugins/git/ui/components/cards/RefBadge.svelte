<script lang="ts">
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import CloudIcon from '@mannie/plugin-sdk/icons/cloud';
  import GitBranchIcon from '@mannie/plugin-sdk/icons/git-branch';
  import TagIcon from '@mannie/plugin-sdk/icons/tag';

  import { parseRefBadge } from '../../utils/git-format.js';

  // Renders a git ref decoration (HEAD -> main, local branch, remote branch,
  // or tag) with a matching icon and single-sided ellipsis truncation inside
  // the badge so long branch names never clip on both ends.
  interface Props {
    readonly reference: string;
  }

  let { reference }: Props = $props();

  const parsed = $derived(parseRefBadge(reference));
  const variant = $derived.by<'default' | 'outline' | 'secondary'>(() => {
    if (parsed.kind === 'head') {
      return 'default';
    }
    if (parsed.kind === 'branch') {
      return 'secondary';
    }
    return 'outline';
  });
</script>

<Badge class="max-w-44 shrink-0 gap-1" title={parsed.raw} {variant}>
  {#if parsed.kind === 'tag'}
    <TagIcon class="size-3 shrink-0" aria-hidden="true" />
  {:else if parsed.kind === 'remote'}
    <CloudIcon class="size-3 shrink-0" aria-hidden="true" />
  {:else}
    <GitBranchIcon class="size-3 shrink-0" aria-hidden="true" />
  {/if}
  <span class="truncate">{parsed.label}</span>
</Badge>
