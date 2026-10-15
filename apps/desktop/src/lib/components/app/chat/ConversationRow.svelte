<script lang="ts">
  import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
  import ShieldAlertIcon from '@lucide/svelte/icons/shield-alert';

  import { resolve } from '$app/paths';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import * as Sidebar from '#lib/components/ui/sidebar/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import type { ConversationSummary } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatActions } from '#lib/state/chat-actions.svelte.js';
  import { conversationQuery, openConversationId } from '#lib/utils/chat-urls.js';

  // One conversation in the sidebar (docs/features/chat/conversation-view.md,
  // layout; projects.md): its title, a mark while its reply waits for the
  // owner's approval or is being written, so the owner finds it from another
  // conversation, and a menu to rename or delete it, which shows on hover and
  // focus. The menu's button lies over room that the row keeps free, so
  // nothing moves when it shows, and the row stays highlighted while the
  // pointer is on the button.
  interface Props {
    conversation: ConversationSummary;
    /** Whether the row belongs to a project's list. */
    nested?: boolean;
  }

  let { conversation, nested = false }: Props = $props();

  const isOpen = $derived(conversation.id === openConversationId());
  // A project's conversations form a sub-list below the project's row.
  const Item = $derived(nested ? Sidebar.MenuSubItem : Sidebar.MenuItem);
  const RowButton = $derived(nested ? Sidebar.MenuSubButton : Sidebar.MenuButton);
</script>

<Item class="group/conversation" data-conversation={conversation.id}>
  <RowButton
    class="pe-8 group-hover/conversation:bg-sidebar-accent group-hover/conversation:text-sidebar-accent-foreground"
    isActive={isOpen}
  >
    {#snippet child({ props })}
      <a
        {...props}
        aria-current={isOpen ? 'page' : undefined}
        href="{resolve('/')}?{conversationQuery(conversation.id)}"
      >
        <span class="truncate" data-content>{conversation.title}</span>
        {#if conversation.awaitingApproval}
          <ShieldAlertIcon class="ms-auto" aria-label={m.chat_conversation_awaiting_approval()} role="img" />
        {:else if conversation.hasActiveRun}
          <Spinner class="ms-auto" aria-label={m.chat_conversation_replying()} />
        {/if}
      </a>
    {/snippet}
  </RowButton>
  <DropdownMenu.Root>
    <DropdownMenu.Trigger>
      {#snippet child({ props })}
        <Button
          {...props}
          class="absolute inset-y-0 inset-e-1 my-auto group-focus-within/conversation:opacity-100 group-hover/conversation:opacity-100 aria-expanded:opacity-100 md:opacity-0"
          data-conversation-menu
          size="icon-xs"
          variant="ghost"
        >
          <EllipsisIcon class="size-4" />
          <span class="sr-only">{m.chat_conversation_menu({ title: conversation.title })}</span>
        </Button>
      {/snippet}
    </DropdownMenu.Trigger>
    <DropdownMenu.Content align="start" side="right">
      <DropdownMenu.Group>
        <DropdownMenu.Item onSelect={() => chatActions.startRename(conversation)}>
          {m.chat_rename()}
        </DropdownMenu.Item>
        <DropdownMenu.Item variant="destructive" onSelect={() => chatActions.startDelete(conversation)}>
          {m.chat_delete()}
        </DropdownMenu.Item>
      </DropdownMenu.Group>
    </DropdownMenu.Content>
  </DropdownMenu.Root>
</Item>
