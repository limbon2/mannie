// The Workers plugin's UI (plugins/workers/docs/plugin.md; ADR-0031): the
// "Workers" group of the sidebar between the projects and the
// conversations, the pairing and removal dialogs, "Pair a worker…" in the
// command palette, the Workers settings section after the Child agents
// section, and the kind of task of its executor, which names a worker
// task's target, its notes and its refusals in the core's task card. Every
// entry shows only while Workers is on, and the list of workers follows the
// plugin's channel only then (ADR-0030, decision 11). The feature Databases
// (plugins/workers/docs/databases.md) adds its entry of the activity bar
// with its view, its sidebar and its dialogs, the kind of mentions that
// names a database after "@", the cards of its tools and "Add database…" in
// the command palette, which show only while Workers and the feature are on
// (ADR-0055, decision 10).
import { definePlugin } from '@mannie/plugin-sdk';
import DatabaseIcon from '@mannie/plugin-sdk/icons/database';
import ListTreeIcon from '@mannie/plugin-sdk/icons/list-tree';
import PencilIcon from '@mannie/plugin-sdk/icons/pencil';
import SearchIcon from '@mannie/plugin-sdk/icons/search';
import ServerIcon from '@mannie/plugin-sdk/icons/server';

import DatabasesDialogs from './components/DatabasesDialogs.svelte';
import DatabasesPanel from './components/DatabasesPanel.svelte';
import DatabasesView from './components/DatabasesView.svelte';
import WorkersDialogs from './components/WorkersDialogs.svelte';
import WorkersPanel from './components/WorkersPanel.svelte';
import WorkersSettingsSection from './components/WorkersSettingsSection.svelte';
import { m } from './generated/messages.js';
import { databaseActions } from './state/database-actions.svelte.js';
import { forgetSqlSessions } from './state/database-sql.svelte.js';
import { databaseList } from './state/databases.svelte.js';
import { WORKERS_SETTINGS_SECTION, workersActions } from './state/workers-actions.svelte.js';
import { workerList } from './state/workers.svelte.js';
import {
  databaseMentionRefusal,
  databaseMentionSummary,
  databaseMentionVariantName,
  databaseMentionWarning,
  databasesToolFailure,
} from './utils/databases-messages.js';
import { DATABASES_FEATURE, DATABASES_VIEW, shownDatabaseKey } from './utils/databases-places.js';
import {
  CHANGE_DATABASE_TOOL,
  changeSummary,
  DESCRIBE_DATABASE_TOOL,
  describeSummary,
  QUERY_DATABASE_TOOL,
  querySummary,
} from './utils/databases-tools.js';
import { workersOffQuestion, workersRefusalMessage, workersStartFailureMessage } from './utils/workers-messages.js';

/** The database whose page the Databases view shows, as the list knows it. */
function shownDatabase(): ReturnType<typeof databaseList.find> {
  const key = shownDatabaseKey();
  return key === null ? undefined : databaseList.find(key);
}

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
    const databases = context.feature(DATABASES_FEATURE);
    databaseList.follow(databases);
    databases.whileOn(
      () => undefined,
      () => {
        databaseActions.closeAll();
        forgetSqlSessions();
      },
    );
    // After Chat and Notebooks in the activity bar.
    databases.slots.navigation.register({
      icon: DatabaseIcon,
      id: DATABASES_VIEW,
      label: m.workers_databases_navigation_label,
      order: 20,
      view: DATABASES_VIEW,
    });
    databases.slots.views.register({
      component: DatabasesView,
      id: DATABASES_VIEW,
      title: () => shownDatabase()?.name ?? m.workers_databases_view_title(),
      titleIsContent: () => shownDatabase() !== undefined,
    });
    databases.slots.sidebarPanels.register({
      activity: DATABASES_VIEW,
      component: DatabasesPanel,
      id: 'workers.databases.panel',
      order: 0,
    });
    databases.slots.overlays.register({ component: DatabasesDialogs, id: 'workers.databases.dialogs' });
    // The provider's ID is the one that the feature's mention provider has.
    databases.slots.mentionKinds.register({
      group: m.workers_databases_mention_group,
      icon: () => DatabaseIcon,
      id: 'workers.databases',
      order: 30,
      refusal: databaseMentionRefusal,
      summary: databaseMentionSummary,
      variantName: databaseMentionVariantName,
      warning: databaseMentionWarning,
    });
    databases.slots.toolKinds.register({
      failure: databasesToolFailure,
      icon: ListTreeIcon,
      id: DESCRIBE_DATABASE_TOOL,
      summary: describeSummary,
    });
    databases.slots.toolKinds.register({
      failure: databasesToolFailure,
      icon: SearchIcon,
      id: QUERY_DATABASE_TOOL,
      summary: querySummary,
    });
    databases.slots.toolKinds.register({
      failure: databasesToolFailure,
      icon: PencilIcon,
      id: CHANGE_DATABASE_TOOL,
      summary: changeSummary,
    });
    databases.slots.commands.register({
      group: m.workers_command_group,
      id: 'workers.databases.add',
      label: m.workers_databases_add,
      order: 10,
      run: () => databaseActions.startAdd(),
    });
  },
});
