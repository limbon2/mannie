<script lang="ts">
  import EllipsisIcon from '@lucide/svelte/icons/ellipsis';
  import FolderIcon from '@lucide/svelte/icons/folder';
  import FolderOpenIcon from '@lucide/svelte/icons/folder-open';
  import ShieldAlertIcon from '@lucide/svelte/icons/shield-alert';
  import SquarePenIcon from '@lucide/svelte/icons/square-pen';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as Collapsible from '#lib/components/ui/collapsible/index.js';
  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import * as Sidebar from '#lib/components/ui/sidebar/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { projectActions } from '#lib/state/project-actions.svelte.js';
  import { sidebarCollapse } from '#lib/state/sidebar-collapse.svelte.js';
  import { newConversationProjectId, openConversationId, startNewConversation } from '#lib/utils/chat-urls.js';
  import { PROJECT_CONVERSATIONS_SHOWN, type ProjectGroup, shownConversations } from '#lib/utils/project-groups.js';

  import ConversationRow from './ConversationRow.svelte';

  // One project in the sidebar (docs/features/chat/projects.md, the sidebar):
  // a row with its folder and name that collapses and expands the project,
  // and below it the project's conversations, a few at first. On hover or
  // focus the row offers a new conversation in the project and a menu with
  // the folder, "Rename…" and "Remove…". Their buttons lie over room that the
  // row keeps free, so nothing moves when they show. A collapsed project still
  // shows when one of its conversations waits for approval or writes a reply.
  let { group }: { group: ProjectGroup } = $props();

  const project = $derived(group.project);
  const open = $derived(sidebarCollapse.isProjectOpen(project.id));
  const listsAll = $derived(sidebarCollapse.listsAll(project.id));
  const shown = $derived(shownConversations(group.conversations, listsAll, openConversationId()));
  const hasMore = $derived(group.conversations.length > PROJECT_CONVERSATIONS_SHOWN);
  const isCurrent = $derived(newConversationProjectId() === project.id);

  function setOpen(value: boolean): void {
    sidebarCollapse.setProjectOpen(project.id, value);
  }
</script>

<Collapsible.Root bind:open={() => open, setOpen}>
  {#snippet child({ props })}
    <Sidebar.MenuItem {...props} data-project={project.id}>
      <div class="group/project relative">
        <Collapsible.Trigger>
          {#snippet child({ props: trigger })}
            <Sidebar.MenuButton
              {...trigger}
              class="pe-14 group-hover/project:bg-sidebar-accent group-hover/project:text-sidebar-accent-foreground"
              data-project-toggle
              isActive={isCurrent}
            >
              {#if open}
                <FolderOpenIcon />
              {:else}
                <FolderIcon />
              {/if}
              <span class="truncate" data-content>{project.name}</span>
              {#if !open && group.awaitingApproval}
                <ShieldAlertIcon class="ms-auto" aria-label={m.chat_project_awaiting_approval()} role="img" />
              {:else if !open && group.replying}
                <Spinner class="ms-auto" aria-label={m.chat_project_replying()} />
              {/if}
            </Sidebar.MenuButton>
          {/snippet}
        </Collapsible.Trigger>
        <div
          class="absolute inset-y-0 inset-e-1 flex items-center group-focus-within/project:opacity-100 group-hover/project:opacity-100 has-aria-expanded:opacity-100 md:opacity-0"
        >
          <Button
            aria-label={m.chat_project_new_conversation({ project: project.name })}
            data-new-conversation-in={project.id}
            size="icon-xs"
            title={m.chat_project_new_conversation({ project: project.name })}
            variant="ghost"
            onclick={() => void startNewConversation(project.id)}
          >
            <SquarePenIcon class="size-4" />
          </Button>
          <DropdownMenu.Root>
            <DropdownMenu.Trigger>
              {#snippet child({ props: trigger })}
                <Button {...trigger} data-project-menu size="icon-xs" variant="ghost">
                  <EllipsisIcon class="size-4" />
                  <span class="sr-only">{m.chat_project_menu({ project: project.name })}</span>
                </Button>
              {/snippet}
            </DropdownMenu.Trigger>
            <DropdownMenu.Content class="w-auto max-w-80" align="start" side="right">
              <DropdownMenu.Label class="font-mono font-normal wrap-break-word" data-content
                >{project.folder}</DropdownMenu.Label
              >
              <DropdownMenu.Separator />
              <DropdownMenu.Group>
                <DropdownMenu.Item onSelect={() => projectActions.startRename(project)}>
                  {m.chat_project_rename()}
                </DropdownMenu.Item>
                <DropdownMenu.Item
                  variant="destructive"
                  onSelect={() => projectActions.startRemove(project, group.conversations.length)}
                >
                  {m.chat_project_remove()}
                </DropdownMenu.Item>
              </DropdownMenu.Group>
            </DropdownMenu.Content>
          </DropdownMenu.Root>
        </div>
      </div>
      <Collapsible.Content>
        <Sidebar.MenuSub>
          {#if group.conversations.length === 0}
            <li class="px-2 py-1 text-xs text-muted-foreground">{m.chat_project_empty()}</li>
          {:else}
            {#each shown as conversation (conversation.id)}
              <ConversationRow {conversation} nested />
            {/each}
            {#if hasMore}
              <Sidebar.MenuSubItem>
                <Sidebar.MenuSubButton class="text-muted-foreground" size="sm">
                  {#snippet child({ props: button })}
                    <button
                      {...button}
                      type="button"
                      onclick={() => sidebarCollapse.setListsAll(project.id, !listsAll)}
                    >
                      <span>{listsAll ? m.chat_project_show_less() : m.chat_project_show_more()}</span>
                    </button>
                  {/snippet}
                </Sidebar.MenuSubButton>
              </Sidebar.MenuSubItem>
            {/if}
          {/if}
        </Sidebar.MenuSub>
      </Collapsible.Content>
    </Sidebar.MenuItem>
  {/snippet}
</Collapsible.Root>
