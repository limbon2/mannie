<script lang="ts">
  import type { ToolKindDetailsProps } from '@mannie/plugin-sdk/slots';

  import { filesOf, patchesOf } from '../../utils/git-data.js';

  import DiffView from './DiffView.svelte';

  // The card of a call of git_diff, git_show or git_commit (ADR-0039): the
  // files that the change touched, each opening to its patch when the data
  // holds it.
  let { call }: ToolKindDetailsProps = $props();

  const files = $derived(filesOf(call.data));
  const patches = $derived(patchesOf(call.data));
</script>

{#if files.length > 0}
  <DiffView {files} open={files.length === 1} {patches} />
{/if}
