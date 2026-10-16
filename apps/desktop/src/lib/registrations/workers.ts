// The workers feature's entries in the shell's slots (docs/features/
// workers/worker-activity.md; ADR-0020): the "Workers" group of the sidebar
// between the projects and the conversations, the pairing and removal
// dialogs, "Pair a worker…" in the command palette, and the Workers settings
// section after the Child agents section. A worker's tasks show through the
// child agents feature's card and chat's conversation panel (ADR-0027).
import ServerIcon from '@lucide/svelte/icons/server';

import WorkersDialogs from '#lib/components/app/workers/WorkersDialogs.svelte';
import WorkersPanel from '#lib/components/app/workers/WorkersPanel.svelte';
import WorkersSettingsSection from '#lib/components/app/workers/WorkersSettingsSection.svelte';
import { m } from '#lib/paraglide/messages.js';
import type { ShellSlots } from '#lib/state/shell-slots.svelte.js';
import { WORKERS_SETTINGS_SECTION, workersActions } from '#lib/state/workers-actions.svelte.js';

/** Registers the workers feature. */
export function registerWorkers(slots: ShellSlots): void {
  slots.sidebarPanels.register({ component: WorkersPanel, id: 'workers.panel', order: 5 });
  slots.overlays.register({ component: WorkersDialogs, id: 'workers.dialogs' });
  slots.commands.register({
    group: m.workers_command_group,
    id: 'workers.pair',
    label: m.workers_command_pair,
    order: 0,
    run: () => workersActions.startPair(),
  });
  slots.settingsSections.register({
    component: WorkersSettingsSection,
    description: m.workers_settings_description,
    icon: ServerIcon,
    id: WORKERS_SETTINGS_SECTION,
    order: 28,
    title: m.workers_settings_title,
  });
}
