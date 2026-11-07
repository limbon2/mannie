<script lang="ts">
  import type { DraftControlProps } from '#lib/types/shell-slots.js';

  import AgentMenu from './AgentMenu.svelte';

  // The agent of a new conversation, in the new conversation pane's message box
  // (ADR-0059, decision 6). The core checks it when the first message is sent.
  let { draft }: DraftControlProps = $props();
</script>

<AgentMenu chosen={draft.agentKey} onChoose={(key: string | null) => draft.chooseAgent(key)} />
