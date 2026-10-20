<script lang="ts">
  import type { ToolKindDetailsProps } from '@mannie/plugin-sdk/slots';

  import { entriesOf } from '../../utils/git-data.js';

  import EntryList from './EntryList.svelte';

  // The card of a call of git_status (ADR-0039): the files that it found,
  // grouped as the model read them.
  let { call }: ToolKindDetailsProps = $props();

  const status = $derived(entriesOf(call.data));
</script>

{#if call.data !== null}
  <EntryList entries={status.entries} total={status.total} />
{/if}
