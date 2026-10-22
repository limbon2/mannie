<script lang="ts">
  import BotIcon from '@lucide/svelte/icons/bot';

  import { Badge } from '#lib/components/ui/badge/index.js';
  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import type { TaskView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { sidePanels } from '#lib/state/side-panels.svelte.js';
  import type { ConversationControlProps } from '#lib/types/shell-slots.js';
  import { CONVERSATION_PANEL } from '#lib/utils/chat-urls.js';
  import { taskStateLabel } from '#lib/utils/child-agents-messages.js';
  import { childAgentsOf, isTaskActive } from '#lib/utils/task-parts.js';

  // The child agents' submenu in the conversation's menu (ADR-0026,
  // ADR-0042; docs/features/child-agents/child-agent-activity.md): it says
  // how many child agents work, and lists the conversation's child agents
  // with their states. Choosing one opens its conversation in a side panel,
  // as its card's "Open conversation" does. Without child agents, the menu
  // shows nothing of it.
  interface ChildAgentGroup {
    readonly heading: string;
    readonly id: string;
    readonly tasks: readonly TaskView[];
  }

  let { opener, turns }: ConversationControlProps = $props();

  const numbers = new Intl.NumberFormat();

  const children = $derived(childAgentsOf(turns));
  const working = $derived(children.filter((task) => isTaskActive(task.state)));
  const ended = $derived(children.filter((task) => !isTaskActive(task.state)));
  /** The submenu's groups that have child agents: the ones that work, then the ones that ended. */
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
    const label = { isContent: true, text: tabLabel(task) };
    void sidePanels.open(CONVERSATION_PANEL, task.conversationId, { label, opener });
  }
</script>

{#if children.length > 0}
  <DropdownMenu.Sub>
    <DropdownMenu.SubTrigger aria-label={name} data-child-agents="">
      <BotIcon />
      <span class="flex-1">{m.child_agents_control_label()}</span>
      {#if working.length > 0}
        <Badge variant="secondary">{numbers.format(working.length)}</Badge>
      {/if}
    </DropdownMenu.SubTrigger>
    <DropdownMenu.SubContent class="w-80">
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
    </DropdownMenu.SubContent>
  </DropdownMenu.Sub>
{/if}
