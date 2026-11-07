<script lang="ts">
  import BotIcon from '@lucide/svelte/icons/bot';
  import PlusIcon from '@lucide/svelte/icons/plus';
  import UsersIcon from '@lucide/svelte/icons/users';
  import { onMount } from 'svelte';

  import { Button } from '#lib/components/ui/button/index.js';
  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import * as Sidebar from '#lib/components/ui/sidebar/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { agentActions } from '#lib/state/agent-actions.svelte.js';
  import { agents } from '#lib/state/agents.svelte.js';
  import { agentFailureMessage, agentName, templateName } from '#lib/utils/agents-messages.js';
  import { openAgents, shownAgentKey } from '#lib/utils/agents-urls.js';

  // The sidebar of the Agents page (docs/features/custom-agents.md): "New agent",
  // and the agents in two groups. Choosing one opens it in the editor.
  const groups = $derived([
    { agents: agents.conversationAgents, icon: BotIcon, id: 'conversation', title: m.agents_panel_conversation() },
    { agents: agents.childAgents, icon: UsersIcon, id: 'child', title: m.agents_panel_child() },
  ]);
  const shown = $derived(shownAgentKey());

  onMount(() => {
    void agents.ensureLoaded();
  });
</script>

<Sidebar.Group class="group-data-[collapsible=icon]:hidden" data-agents-panel="">
  <Sidebar.GroupContent class="flex flex-col gap-4">
    <DropdownMenu.Root>
      <DropdownMenu.Trigger>
        {#snippet child({ props })}
          <Button {...props} class="w-full justify-start" data-agents-new="" size="sm" variant="outline">
            <PlusIcon data-icon="inline-start" />
            {m.agents_panel_new()}
          </Button>
        {/snippet}
      </DropdownMenu.Trigger>
      <DropdownMenu.Content class="w-64" align="start">
        {#each agents.options?.templates ?? [] as template (template.id)}
          <DropdownMenu.Item
            data-agents-template={template.id}
            onSelect={() => void agentActions.start(template.draft)}
          >
            {templateName(template.id)}
          </DropdownMenu.Item>
        {/each}
      </DropdownMenu.Content>
    </DropdownMenu.Root>
    {#if agents.status === 'loading'}
      <Sidebar.Menu aria-busy="true">
        {#each [0, 1, 2] as placeholder (placeholder)}
          <Sidebar.MenuItem>
            <Sidebar.MenuSkeleton showIcon />
          </Sidebar.MenuItem>
        {/each}
      </Sidebar.Menu>
      <span class="sr-only" role="status">{m.agents_panel_loading()}</span>
    {:else if agents.status === 'error'}
      <div class="flex flex-col items-start gap-2 px-2 text-xs">
        <p class="text-destructive">{agents.failure === null ? '' : agentFailureMessage(agents.failure)}</p>
        <Button size="sm" variant="outline" onclick={() => void agents.reload()}>{m.chat_try_again()}</Button>
      </div>
    {:else}
      {#each groups as group (group.id)}
        <div class="flex flex-col gap-1" data-agents-group={group.id}>
          <p class="px-2 text-xs font-medium text-muted-foreground">{group.title}</p>
          <Sidebar.Menu>
            {#each group.agents as agent (agent.key)}
              <Sidebar.MenuItem>
                <Sidebar.MenuButton
                  data-agent-row={agent.key}
                  isActive={shown === agent.key}
                  onclick={() => void openAgents(agent.key)}
                >
                  <group.icon />
                  <span class="truncate" data-content={agent.builtin ? undefined : ''} dir="auto"
                    >{agentName(agent)}</span
                  >
                </Sidebar.MenuButton>
              </Sidebar.MenuItem>
            {/each}
          </Sidebar.Menu>
        </div>
      {/each}
    {/if}
  </Sidebar.GroupContent>
</Sidebar.Group>
