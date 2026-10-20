<script lang="ts">
  import type { ToolKindDetailsProps } from '@mannie/plugin-sdk/slots';

  import { m } from '../../generated/messages.js';
  import { refsOf } from '../../utils/git-data.js';

  import RefList from './RefList.svelte';

  // The card of a call of git_refs (ADR-0039): the branches, remote
  // branches and tags that it listed.
  let { call }: ToolKindDetailsProps = $props();
</script>

<div class="flex flex-col gap-3">
  <RefList heading={m.git_refs_branches()} refs={refsOf(call.data, 'branches')} />
  <RefList heading={m.git_refs_remote_branches()} refs={refsOf(call.data, 'remoteBranches')} />
  <RefList heading={m.git_refs_tags()} refs={refsOf(call.data, 'tags')} />
</div>
