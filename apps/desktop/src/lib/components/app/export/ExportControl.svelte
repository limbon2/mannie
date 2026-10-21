<script lang="ts">
  import DownloadIcon from '@lucide/svelte/icons/download';

  import { Button } from '#lib/components/ui/button/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { exportActions } from '#lib/state/export-actions.svelte.js';
  import type { ConversationControlProps } from '#lib/types/shell-slots.js';

  let { conversation }: ConversationControlProps = $props();
  const active = $derived(exportActions.busy && exportActions.conversationId === conversation.id);
  const label = $derived(active ? m.export_working() : m.export_action());
</script>

<Button
  aria-busy={active}
  aria-label={label}
  data-export-conversation={conversation.id}
  disabled={exportActions.busy}
  size="icon-sm"
  title={label}
  variant="ghost"
  onclick={() => void exportActions.start(conversation.id)}
>
  {#if active}
    <Spinner aria-hidden="true" />
  {:else}
    <DownloadIcon />
  {/if}
</Button>
