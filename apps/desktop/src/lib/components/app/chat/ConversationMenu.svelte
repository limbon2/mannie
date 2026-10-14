<script lang="ts">
  import EllipsisIcon from '@lucide/svelte/icons/ellipsis';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import type { ConversationSummary } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatActions } from '#lib/state/chat-actions.svelte.js';

  // The open conversation's menu: rename, compact and delete it.
  let { conversation }: { conversation: ConversationSummary } = $props();

  const compacting = $derived(chatActions.compacting.includes(conversation.id));
</script>

<DropdownMenu.Root>
  <DropdownMenu.Trigger>
    {#snippet child({ props })}
      <Button
        {...props}
        aria-label={m.chat_conversation_menu({ title: conversation.title })}
        size="icon-sm"
        variant="ghost"
      >
        <EllipsisIcon />
      </Button>
    {/snippet}
  </DropdownMenu.Trigger>
  <DropdownMenu.Content align="end">
    <DropdownMenu.Group>
      <DropdownMenu.Item onSelect={() => chatActions.startRename(conversation)}>
        {m.chat_rename()}
      </DropdownMenu.Item>
      <DropdownMenu.Item disabled={compacting} onSelect={() => void chatActions.compact(conversation.id)}>
        {m.chat_compact_now()}
      </DropdownMenu.Item>
    </DropdownMenu.Group>
    <DropdownMenu.Separator />
    <DropdownMenu.Group>
      <DropdownMenu.Item variant="destructive" onSelect={() => chatActions.startDelete(conversation)}>
        {m.chat_delete()}
      </DropdownMenu.Item>
    </DropdownMenu.Group>
  </DropdownMenu.Content>
</DropdownMenu.Root>
