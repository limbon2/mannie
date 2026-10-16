// The Workers plugin's UI (docs/features/workers/plugin.md; ADR-0031): the
// "Workers" group of the sidebar between the projects and the
// conversations, the pairing and removal dialogs, "Pair a worker…" in the
// command palette, the Workers settings section after the Child agents
// section, and the kind of task of its executor, which names a worker
// task's target, its notes and its refusals in the core's task card. Every
// entry shows only while Workers is on (ADR-0030, decision 11).
import ServerIcon from '@lucide/svelte/icons/server';

import WorkersDialogs from '#lib/components/app/workers/WorkersDialogs.svelte';
import WorkersPanel from '#lib/components/app/workers/WorkersPanel.svelte';
import WorkersSettingsSection from '#lib/components/app/workers/WorkersSettingsSection.svelte';
import { m } from '#lib/paraglide/messages.js';
import { WORKERS_SETTINGS_SECTION, workersActions } from '#lib/state/workers-actions.svelte.js';
import { WORKERS_PLUGIN } from '#lib/state/workers.svelte.js';
import type { PluginRegistration } from '#lib/types/plugins.js';
import { workersRefusalMessage, workersStartFailureMessage } from '#lib/utils/workers-messages.js';

/** The Workers plugin. */
export const workersPlugin: PluginRegistration = {
  description: m.workers_plugin_description,
  failure: workersStartFailureMessage,
  id: WORKERS_PLUGIN,
  name: m.workers_plugin_name,
  offQuestion: (work) => (work.activeTasks === 0 ? null : m.workers_plugin_off_question({ count: work.activeTasks })),
  register(slots) {
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
    slots.taskKinds.register({
      detached: m.workers_task_detached,
      icon: ServerIcon,
      id: WORKERS_PLUGIN,
      readOnly: m.workers_task_read_only,
      refusal: workersRefusalMessage,
      targetPrefix: m.workers_task_target_prefix,
      tool: 'delegate_task',
      untitledCall: m.workers_task_untitled_call,
      waiting: (count) => m.workers_task_waiting({ count }),
    });
  },
};
