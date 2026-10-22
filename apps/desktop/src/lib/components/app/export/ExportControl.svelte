<script lang="ts">
  import DownloadIcon from '@lucide/svelte/icons/download';

  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { exportActions } from '#lib/state/export-actions.svelte.js';
  import type { ConversationControlProps } from '#lib/types/shell-slots.js';

  // The export's item in the conversation's menu (ADR-0040, ADR-0042;
  // docs/features/conversation-export.md). While an export runs, the item
  // says so and waits, for this conversation and every other.
  let { conversation }: ConversationControlProps = $props();

  const active = $derived(exportActions.busy && exportActions.conversationId === conversation.id);
  const label = $derived(active ? m.export_working() : m.export_action());
</script>

<DropdownMenu.Item
  aria-busy={active}
  data-export-conversation={conversation.id}
  disabled={exportActions.busy}
  onSelect={() => void exportActions.start(conversation.id)}
>
  {#if active}
    <Spinner aria-hidden="true" />
  {:else}
    <DownloadIcon />
  {/if}
  {label}
</DropdownMenu.Item>
