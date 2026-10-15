<script lang="ts">
  import PlusIcon from '@lucide/svelte/icons/plus';
  import { onMount } from 'svelte';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as Sidebar from '#lib/components/ui/sidebar/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { conversationList } from '#lib/state/conversation-list.svelte.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';
  import { startNewConversation } from '#lib/utils/chat-urls.js';

  import ConversationRow from './ConversationRow.svelte';

  // The "Conversations" group of the sidebar (ADR-0020, a sidebar panel): the
  // general conversations, outside every project, most recent first, with a
  // menu to rename or delete each. The projects' conversations are in the
  // "Projects" group above (docs/features/chat/projects.md).
  const general = $derived(conversationList.grouped.general);

  onMount(() => {
    conversationList.start();
  });
</script>

<Sidebar.Group class="min-h-0 group-data-[collapsible=icon]:hidden">
  <Sidebar.GroupLabel>{m.chat_conversations()}</Sidebar.GroupLabel>
  <Sidebar.GroupAction
    data-new-conversation
    title={m.chat_new_conversation()}
    onclick={() => void startNewConversation()}
  >
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
    {:else if general.length === 0}
      <p class="px-2 text-xs text-muted-foreground">{m.chat_conversations_outside_projects_empty()}</p>
    {:else}
      <Sidebar.Menu>
        {#each general as conversation (conversation.id)}
          <ConversationRow {conversation} />
        {/each}
      </Sidebar.Menu>
    {/if}
  </Sidebar.GroupContent>
</Sidebar.Group>
