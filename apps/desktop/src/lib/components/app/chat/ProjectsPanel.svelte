<script lang="ts">
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';
  import PlusIcon from '@lucide/svelte/icons/plus';
  import { onMount } from 'svelte';

  import * as Collapsible from '#lib/components/ui/collapsible/index.js';
  import * as Sidebar from '#lib/components/ui/sidebar/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { conversationList, type ConversationMove } from '#lib/state/conversation-list.svelte.js';
  import { projectActions } from '#lib/state/project-actions.svelte.js';
  import { sidebarCollapse } from '#lib/state/sidebar-collapse.svelte.js';
  import { startedConversationId } from '#lib/state/started-conversation.js';
  import { openConversationId } from '#lib/utils/chat-urls.js';

  import ProjectRow from './ProjectRow.svelte';

  // The "Projects" group of the sidebar (ADR-0020, a sidebar panel;
  // docs/features/chat/projects.md): above the general conversations, with
  // "+", which adds a folder through the system's folder dialog, and the
  // projects, the most recently active first. The group collapses from its
  // heading. When the conversation list cannot be read, the "Conversations"
  // group below says why.
  const groups = $derived(conversationList.grouped.projects);

  function setOpen(open: boolean): void {
    sidebarCollapse.setProjectsOpen(open);
  }

  /**
   * When the open conversation moves into a collapsed project, the project
   * expands, so the owner sees where the conversation went. The conversation
   * that a first message started counts as open until the view shows it,
   * since its reply can move it before then. Opening or reloading a
   * conversation keeps what the owner collapsed.
   */
  function revealMove(move: ConversationMove): void {
    const openId = openConversationId();
    const isOpen = move.conversationId === openId;
    const isOpening = openId === null && move.conversationId === startedConversationId();
    if (isOpen || isOpening) {
      sidebarCollapse.reveal(move.projectId);
    }
  }

  onMount(() => {
    conversationList.start();
    return conversationList.onMove(revealMove);
  });
</script>

<Collapsible.Root
  class="group/collapsible group-data-[collapsible=icon]:hidden"
  bind:open={() => !sidebarCollapse.projectsCollapsed, setOpen}
>
  <Sidebar.Group>
    <Sidebar.GroupLabel class="gap-1 hover:text-sidebar-foreground">
      {#snippet child({ props })}
        <Collapsible.Trigger {...props}>
          <span>{m.chat_projects()}</span>
          <ChevronDownIcon class="transition-transform group-data-[state=closed]/collapsible:-rotate-90" />
        </Collapsible.Trigger>
      {/snippet}
    </Sidebar.GroupLabel>
    <Sidebar.GroupAction
      data-add-project
      disabled={projectActions.adding}
      title={m.chat_project_add()}
      onclick={() => void projectActions.add()}
    >
      <PlusIcon />
      <span class="sr-only">{m.chat_project_add()}</span>
    </Sidebar.GroupAction>
    <Collapsible.Content>
      <Sidebar.GroupContent>
        {#if conversationList.status === 'loading'}
          <Sidebar.Menu aria-busy="true">
            {#each [0, 1] as placeholder (placeholder)}
              <Sidebar.MenuItem>
                <Sidebar.MenuSkeleton showIcon />
              </Sidebar.MenuItem>
            {/each}
          </Sidebar.Menu>
        {:else if conversationList.status === 'ready'}
          {#if groups.length === 0}
            <p class="px-2 text-xs text-muted-foreground">{m.chat_projects_empty()}</p>
          {:else}
            <Sidebar.Menu>
              {#each groups as group (group.project.id)}
                <ProjectRow {group} />
              {/each}
            </Sidebar.Menu>
          {/if}
        {/if}
      </Sidebar.GroupContent>
    </Collapsible.Content>
  </Sidebar.Group>
</Collapsible.Root>
