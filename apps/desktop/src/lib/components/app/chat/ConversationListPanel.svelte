<script lang="ts">
  import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
  import PlusIcon from '@lucide/svelte/icons/plus';
  import { onMount } from 'svelte';

  import { resolve } from '$app/paths';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import * as Sidebar from '#lib/components/ui/sidebar/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import type { ConversationSummary } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatActions } from '#lib/state/chat-actions.svelte.js';
  import { conversationList } from '#lib/state/conversation-list.svelte.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';
  import { conversationQuery, openConversationId, startNewConversation } from '#lib/utils/chat-urls.js';

  // The conversation list in the sidebar (ADR-0020, a sidebar panel): the
  // conversations, most recent first, with a menu to rename or delete each.
  const openId = $derived(openConversationId());

  function isOpen(conversation: ConversationSummary): boolean {
    return conversation.id === openId;
  }

  onMount(() => {
    conversationList.start();
  });
</script>

<Sidebar.Group class="min-h-0 group-data-[collapsible=icon]:hidden">
  <Sidebar.GroupLabel>{m.chat_conversations()}</Sidebar.GroupLabel>
  <Sidebar.GroupAction title={m.chat_new_conversation()} onclick={() => void startNewConversation()}>
    <PlusIcon />
    <span class="sr-only">{m.chat_new_conversation()}</span>
  </Sidebar.GroupAction>
  <Sidebar.GroupContent>
    {#if conversationList.status === 'loading'}
      <Sidebar.Menu aria-busy="true">
        {#each [0, 1, 2] as placeholder (placeholder)}
          <Sidebar.MenuItem>
            <Sidebar.MenuSkeleton />
          </Sidebar.MenuItem>
        {/each}
      </Sidebar.Menu>
      <span class="sr-only" role="status">{m.chat_conversations_loading()}</span>
    {:else if conversationList.status === 'error'}
      <div class="flex flex-col items-start gap-2 px-2 text-xs">
        <p class="text-destructive">
          {conversationList.failure === null ? '' : chatFailureMessage(conversationList.failure)}
        </p>
        <Button size="sm" variant="outline" onclick={() => conversationList.retry()}>{m.chat_try_again()}</Button>
      </div>
    {:else if conversationList.conversations.length === 0}
      <p class="px-2 text-xs text-muted-foreground">{m.chat_conversations_empty()}</p>
    {:else}
      <Sidebar.Menu>
        {#each conversationList.conversations as conversation (conversation.id)}
          <Sidebar.MenuItem>
            <Sidebar.MenuButton isActive={isOpen(conversation)}>
              {#snippet child({ props })}
                <a
                  {...props}
                  aria-current={isOpen(conversation) ? 'page' : undefined}
                  href="{resolve('/')}?{conversationQuery(conversation.id)}"
                >
                  <span class="truncate" data-content>{conversation.title}</span>
                  {#if conversation.hasActiveRun}
                    <Spinner class="ms-auto" aria-label={m.chat_conversation_replying()} />
                  {/if}
                </a>
              {/snippet}
            </Sidebar.MenuButton>
            <DropdownMenu.Root>
              <DropdownMenu.Trigger>
                {#snippet child({ props })}
                  <Sidebar.MenuAction {...props} showOnHover>
                    <EllipsisIcon />
                    <span class="sr-only">{m.chat_conversation_menu({ title: conversation.title })}</span>
                  </Sidebar.MenuAction>
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
          </Sidebar.MenuItem>
        {/each}
      </Sidebar.Menu>
    {/if}
  </Sidebar.GroupContent>
</Sidebar.Group>
