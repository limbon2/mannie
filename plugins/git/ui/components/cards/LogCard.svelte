<script lang="ts">
  import type { ToolKindDetailsProps } from '@mannie/plugin-sdk/slots';

  import { loggedCommitsOf } from '../../utils/git-data.js';

  import CommitList from './CommitList.svelte';

  // The card of a call of git_log (ADR-0039): the commits that it listed,
  // the commits that changed the lines that it followed, or the commits
  // that a reflog's entries name.
  let { call }: ToolKindDetailsProps = $props();

  const commits = $derived(loggedCommitsOf(call.data));
</script>

{#if commits.length > 0}
  <CommitList {commits} />
{/if}
