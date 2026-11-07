// The plans feature's entries in the shell's slots (docs/features/plans.md;
// ADR-0060): the group of Chat's sidebar, the side panel that shows a plan, and
// the mention kind of the "@" list. The tool's row and card belong to the core's
// `tool_call` renderer, as the card of `ask_owner` does.
import ClipboardListIcon from '@lucide/svelte/icons/clipboard-list';

import PlanPanel from '#lib/components/app/plans/PlanPanel.svelte';
import PlansPanel from '#lib/components/app/plans/PlansPanel.svelte';
import { m } from '#lib/paraglide/messages.js';
import { PLANS_PROVIDER } from '#lib/state/plans.svelte.js';
import type { ShellSlots } from '#lib/state/shell-slots.svelte.js';
import {
  PLAN_PANEL,
  planMentionIcon,
  planMentionRefusal,
  planMentionSummary,
  planMentionVariantName,
} from '#lib/utils/plans-messages.js';

/** Registers the plans feature. */
export function registerPlans(slots: ShellSlots): void {
  slots.sidebarPanels.register({ component: PlansPanel, id: 'chat.plans', order: 5 });
  slots.sidePanels.register({
    component: PlanPanel,
    defaultLabel: m.plans_panel_label,
    icon: ClipboardListIcon,
    id: PLAN_PANEL,
  });
  slots.mentionKinds.register({
    group: m.plans_mention_group,
    icon: planMentionIcon,
    id: PLANS_PROVIDER,
    order: 5,
    refusal: planMentionRefusal,
    summary: planMentionSummary,
    variantName: planMentionVariantName,
    // The provider warns about no entry.
    warning: () => null,
  });
}
