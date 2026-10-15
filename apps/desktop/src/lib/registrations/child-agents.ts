// The child agents feature's entries in the shell's slots (docs/features/
// child-agents/child-agent-activity.md; ADR-0020): the renderer of the parts
// of task tools, the button in a conversation's toolbar that lists its child
// agents (ADR-0026), and the Child agents settings section. A card and the
// button open a child's conversation in chat's conversation panel
// (ADR-0025).
import BotIcon from '@lucide/svelte/icons/bot';

import ChildAgentsControl from '#lib/components/app/child-agents/ChildAgentsControl.svelte';
import ChildAgentsSettingsSection from '#lib/components/app/child-agents/ChildAgentsSettingsSection.svelte';
import TaskPart from '#lib/components/app/child-agents/TaskPart.svelte';
import { m } from '#lib/paraglide/messages.js';
import type { ShellSlots } from '#lib/state/shell-slots.svelte.js';

/** Registers the child agents feature. */
export function registerChildAgents(slots: ShellSlots): void {
  slots.partRenderers.register({ component: TaskPart, id: 'task' });
  slots.conversationControls.register({ component: ChildAgentsControl, id: 'child-agents.control', order: 10 });
  slots.settingsSections.register({
    component: ChildAgentsSettingsSection,
    description: m.child_agents_settings_description,
    icon: BotIcon,
    id: 'child-agents.settings',
    order: 27,
    title: m.child_agents_settings_title,
  });
}
