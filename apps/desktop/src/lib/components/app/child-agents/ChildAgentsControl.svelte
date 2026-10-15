<script lang="ts">
  import BotIcon from '@lucide/svelte/icons/bot';

  import { Badge } from '#lib/components/ui/badge/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import type { TaskView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { sidePanels } from '#lib/state/side-panels.svelte.js';
  import type { ConversationControlProps } from '#lib/types/shell-slots.js';
  import { CONVERSATION_PANEL } from '#lib/utils/chat-urls.js';
  import { taskStateLabel } from '#lib/utils/child-agents-messages.js';
  import { childAgentsOf, isTaskActive } from '#lib/utils/task-parts.js';

  // The child agents' button in a conversation's toolbar (ADR-0026;
  // docs/features/child-agents/child-agent-activity.md): it says how many
  // child agents work, and its menu lists the conversation's child agents
  // with their states. Choosing one opens its conversation in a side panel,
  // as its card's "Open conversation" does. Without child agents, the
  // toolbar shows nothing of it.
  interface ChildAgentGroup {
    readonly heading: string;
    readonly id: string;
    readonly tasks: readonly TaskView[];
  }

  let { turns }: ConversationControlProps = $props();

  const numbers = new Intl.NumberFormat();

  let trigger = $state<HTMLElement | null>(null);
  /** Whether the latest choice opened a panel, whose heading then keeps the focus that the menu would give back. */
  let openedPanel = false;

  const children = $derived(childAgentsOf(turns));
  const working = $derived(children.filter((task) => isTaskActive(task.state)));
  const ended = $derived(children.filter((task) => !isTaskActive(task.state)));
  /** The menu's groups that have child agents: the ones that work, then the ones that ended. */
  const groups: ChildAgentGroup[] = $derived(
    [
      { heading: m.child_agents_control_working(), id: 'working', tasks: working },
      { heading: m.child_agents_control_ended(), id: 'ended', tasks: ended },
    ].filter((group) => group.tasks.length > 0),
  );
  const name = $derived(
    working.length === 0
      ? m.child_agents_control_label()
      : m.child_agents_control_label_working({ count: working.length }),
  );

  function tabLabel(task: TaskView): string {
    return m.child_agents_tab_label({ number: String(task.number), title: task.title });
  }

  function openConversation(task: TaskView): void {
    openedPanel = true;
    const label = { isContent: true, text: tabLabel(task) };
    void sidePanels.open(CONVERSATION_PANEL, task.conversationId, { label, opener: trigger });
  }

  function keepPanelFocus(event: Event): void {
    if (openedPanel) {
      event.preventDefault();
      openedPanel = false;
    }
  }
</script>

{#if children.length > 0}
  <DropdownMenu.Root>
    <DropdownMenu.Trigger bind:ref={trigger}>
      {#snippet child({ props })}
        <Button {...props} aria-label={name} data-child-agents="" size="sm" variant="ghost">
          <BotIcon data-icon="inline-start" />
          {m.child_agents_control_label()}
          {#if working.length > 0}
            <Badge variant="secondary">{numbers.format(working.length)}</Badge>
          {/if}
        </Button>
      {/snippet}
    </DropdownMenu.Trigger>
    <DropdownMenu.Content class="w-80" align="end" onCloseAutoFocus={keepPanelFocus}>
      {#each groups as group, index (group.id)}
        {#if index > 0}
          <DropdownMenu.Separator />
        {/if}
        <DropdownMenu.Group>
          <DropdownMenu.GroupHeading>{group.heading}</DropdownMenu.GroupHeading>
          {#each group.tasks as task (task.conversationId)}
            <DropdownMenu.Item textValue={tabLabel(task)} onSelect={() => openConversation(task)}>
              <span class="min-w-0 flex-1 truncate" data-content dir="auto">{tabLabel(task)}</span>
              <span class="shrink-0 text-xs text-muted-foreground">{taskStateLabel(task)}</span>
            </DropdownMenu.Item>
          {/each}
        </DropdownMenu.Group>
      {/each}
    </DropdownMenu.Content>
  </DropdownMenu.Root>
{/if}
