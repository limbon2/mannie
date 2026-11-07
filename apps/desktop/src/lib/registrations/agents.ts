// The custom agents feature's entries in the shell's slots (docs/features/
// custom-agents.md; ADR-0059): the entry of the activity bar with its view and
// sidebar panel, and the picker in the message box, which also shows in the new
// conversation pane.
import BotIcon from '@lucide/svelte/icons/bot';

import AgentDraftPicker from '#lib/components/app/agents/AgentDraftPicker.svelte';
import AgentPicker from '#lib/components/app/agents/AgentPicker.svelte';
import AgentsPanel from '#lib/components/app/agents/AgentsPanel.svelte';
import AgentsView from '#lib/components/app/agents/AgentsView.svelte';
import { m } from '#lib/paraglide/messages.js';
import type { ShellSlots } from '#lib/state/shell-slots.svelte.js';
import { AGENTS_VIEW } from '#lib/utils/agents-urls.js';

/** Registers the custom agents feature. */
export function registerAgents(slots: ShellSlots): void {
  slots.navigation.register({
    icon: BotIcon,
    id: AGENTS_VIEW,
    label: m.agents_navigation_label,
    order: 5,
    view: AGENTS_VIEW,
  });
  slots.views.register({
    component: AgentsView,
    id: AGENTS_VIEW,
    title: m.agents_view_title,
  });
  slots.sidebarPanels.register({
    activity: AGENTS_VIEW,
    component: AgentsPanel,
    id: 'agents.agents',
    order: 0,
  });
  slots.conversationControls.register({
    component: AgentPicker,
    draft: AgentDraftPicker,
    id: 'agents.picker',
    order: 0,
    place: 'message-box',
  });
}
