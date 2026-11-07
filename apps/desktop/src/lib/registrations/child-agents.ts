// The child agents feature's entries in the shell's slots (docs/features/
// child-agents/child-agent-activity.md; ADR-0020): the renderer of the parts
// of task tools, the submenu of a conversation's menu that lists its child
// agents and counts the working ones (ADR-0026, ADR-0042), and the Child
// agents settings section. A card and the submenu open a child's
// conversation in chat's conversation panel (ADR-0025).
import BotIcon from '@lucide/svelte/icons/bot';

import ChildAgentsAutoOpen from '#lib/components/app/child-agents/ChildAgentsAutoOpen.svelte';
import ChildAgentsControl from '#lib/components/app/child-agents/ChildAgentsControl.svelte';
import ChildAgentsPanel from '#lib/components/app/child-agents/ChildAgentsPanel.svelte';
import ChildAgentsSettingsSection from '#lib/components/app/child-agents/ChildAgentsSettingsSection.svelte';
import TaskPart from '#lib/components/app/child-agents/TaskPart.svelte';
import { m } from '#lib/paraglide/messages.js';
import type { ShellSlots } from '#lib/state/shell-slots.svelte.js';
import { CHILD_AGENTS_PANEL } from '#lib/utils/child-agents-messages.js';
import { childAgentsOf, isTaskActive } from '#lib/utils/task-parts.js';

/** Registers the child agents feature. */
export function registerChildAgents(slots: ShellSlots): void {
  slots.partRenderers.register({ component: TaskPart, id: 'task' });
  slots.conversationControls.register({
    component: ChildAgentsControl,
    id: 'child-agents.control',
    order: 10,
    place: 'menu',
    running: ({ turns }) => childAgentsOf(turns).filter((task) => isTaskActive(task.state)).length,
  });
  // Opens the panel below on a first task of a custom agent; it shows nothing.
  slots.conversationControls.register({
    component: ChildAgentsAutoOpen,
    id: 'child-agents.auto-open',
    order: 90,
    place: 'message-box',
  });
  slots.sidePanels.register({
    component: ChildAgentsPanel,
    defaultLabel: m.child_agents_panel_label,
    icon: BotIcon,
    id: CHILD_AGENTS_PANEL,
  });
  slots.settingsSections.register({
    component: ChildAgentsSettingsSection,
    description: m.child_agents_settings_description,
    icon: BotIcon,
    id: 'child-agents.settings',
    order: 27,
    title: m.child_agents_settings_title,
  });
}
