// The Workers plugin's UI (plugins/workers/docs/plugin.md; ADR-0031): the
// "Workers" group of the sidebar between the projects and the
// conversations, the pairing and removal dialogs, "Pair a worker…" in the
// command palette, the Workers settings section after the Child agents
// section, and the kind of task of its executor, which names a worker
// task's target, its notes and its refusals in the core's task card. Every
// entry shows only while Workers is on, and the list of workers follows the
// plugin's channel only then (ADR-0030, decision 11).
import { definePlugin } from '@mannie/plugin-sdk';
import ServerIcon from '@mannie/plugin-sdk/icons/server';

import WorkersDialogs from './components/WorkersDialogs.svelte';
import WorkersPanel from './components/WorkersPanel.svelte';
import WorkersSettingsSection from './components/WorkersSettingsSection.svelte';
import { m } from './generated/messages.js';
import { WORKERS_SETTINGS_SECTION, workersActions } from './state/workers-actions.svelte.js';
import { workerList } from './state/workers.svelte.js';
import { workersOffQuestion, workersRefusalMessage, workersStartFailureMessage } from './utils/workers-messages.js';

export default definePlugin({
  failure: workersStartFailureMessage,
  offQuestion: workersOffQuestion,
  register(context) {
    const { slots } = context;
    workerList.follow(context);
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
    // The executor's kind is the plugin's ID.
    slots.taskKinds.register({
      detached: m.workers_task_detached,
      icon: ServerIcon,
      id: context.id,
      readOnly: m.workers_task_read_only,
      refusal: workersRefusalMessage,
      targetPrefix: m.workers_task_target_prefix,
      tool: 'delegate_task',
      untitledCall: m.workers_task_untitled_call,
      waiting: (count) => m.workers_task_waiting({ count }),
    });
  },
});
