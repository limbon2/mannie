<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Collapsible from '@mannie/plugin-sdk/components/collapsible';
  import * as Sidebar from '@mannie/plugin-sdk/components/sidebar';
  import BookOpenIcon from '@mannie/plugin-sdk/icons/book-open';
  import ChevronDownIcon from '@mannie/plugin-sdk/icons/chevron-down';
  import MessageSquarePlusIcon from '@mannie/plugin-sdk/icons/message-square-plus';
  import MessagesSquareIcon from '@mannie/plugin-sdk/icons/messages-square';
  import TrashIcon from '@mannie/plugin-sdk/icons/trash';
  import { openConversation } from '@mannie/plugin-sdk/navigation';

  import type { NotebookView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { notebookActions } from '../state/notebook-actions.svelte.js';
  import { panelRows } from '../state/panel-rows.svelte.js';
  import { chatsOf } from '../utils/notebooks-chats.js';
  import { openNotebook, shownNotebookId } from '../utils/notebooks-places.js';

  // One notebook in the sidebar (plugins/notebooks/docs/README.md, chats):
  // its row opens its page, and its toggle shows "New chat" and the
  // notebook's own chats, the most recently changed first, each of which
  // opens in the chat view, with a button that deletes it after asking. The
  // button lies over room that the chat's row keeps free, and shows on hover
  // and focus, so nothing moves when it shows. The row opens as its
  // notebook's page does, and
  // keeps its state while a chat shows. The buttons take the sidebar's
  // width, so a long title ends in "…" instead of running past it.
  let { notebook }: { notebook: NotebookView } = $props();

  const isCurrent = $derived(shownNotebookId() === notebook.id);
  const chats = $derived(chatsOf(notebook.id));
</script>

<Collapsible.Root bind:open={() => panelRows.isOpen(notebook.id), (open: boolean) => panelRows.set(notebook.id, open)}>
  {#snippet child({ props })}
    <Sidebar.MenuItem {...props} data-notebooks-panel-notebook={notebook.id}>
      <Sidebar.MenuButton isActive={isCurrent} onclick={() => void openNotebook(notebook.id)}>
        <BookOpenIcon />
        <span class="truncate" data-content dir="auto">{notebook.name}</span>
      </Sidebar.MenuButton>
      <Collapsible.Trigger>
        {#snippet child({ props: trigger })}
          <Sidebar.MenuAction
            {...trigger}
            class="group/toggle"
            aria-label={m.notebooks_panel_toggle({ name: notebook.name })}
            data-notebooks-panel-toggle=""
          >
            <ChevronDownIcon
              class="transition-transform group-data-[state=closed]/toggle:-rotate-90 rtl:group-data-[state=closed]/toggle:rotate-90"
            />
          </Sidebar.MenuAction>
        {/snippet}
      </Collapsible.Trigger>
      <Collapsible.Content>
        <Sidebar.MenuSub>
          <Sidebar.MenuSubItem>
            <Sidebar.MenuSubButton class="w-full">
              {#snippet child({ props: button })}
                <button
                  {...button}
                  data-notebooks-panel-new-chat=""
                  type="button"
                  onclick={() => void notebookActions.newChat(notebook)}
                >
                  <MessageSquarePlusIcon />
                  <span class="truncate">{m.notebooks_new_chat()}</span>
                </button>
              {/snippet}
            </Sidebar.MenuSubButton>
          </Sidebar.MenuSubItem>
          {#each chats as chat (chat.id)}
            <Sidebar.MenuSubItem class="group/chat">
              <Sidebar.MenuSubButton class="w-full pe-8">
                {#snippet child({ props: button })}
                  <button
                    {...button}
                    data-notebooks-panel-chat={chat.id}
                    type="button"
                    onclick={() => void openConversation(chat.id)}
                  >
                    <MessagesSquareIcon />
                    <span class="truncate" data-content dir="auto">{chat.title}</span>
                  </button>
                {/snippet}
              </Sidebar.MenuSubButton>
              <div
                class="absolute inset-y-0 inset-e-1 flex items-center group-focus-within/chat:opacity-100 group-hover/chat:opacity-100 md:opacity-0"
              >
                <Button
                  aria-label={m.notebooks_chat_delete({ title: chat.title })}
                  data-notebooks-panel-chat-delete={chat.id}
                  size="icon-xs"
                  title={m.notebooks_chat_delete({ title: chat.title })}
                  variant="ghost"
                  onclick={() => notebookActions.startDeleteChat(chat, notebook.id)}
                >
                  <TrashIcon class="size-3.5" />
                </Button>
              </div>
            </Sidebar.MenuSubItem>
          {/each}
        </Sidebar.MenuSub>
      </Collapsible.Content>
    </Sidebar.MenuItem>
  {/snippet}
</Collapsible.Root>
