// The Notebooks plugin's UI (plugins/notebooks/docs/README.md; ADR-0045,
// ADR-0046, ADR-0049 to ADR-0052): the Notebooks entry of the activity bar
// with its view and its group of the sidebar, which lists each notebook's
// chats, the dialogs and the viewer of sources, the kind of context that
// names notebooks and opens their passages, the kind of mentions that
// attaches a notebook after "@", the cards of the plugin's tools, "New
// notebook…" in the command palette and the Notebooks settings. Every entry
// shows only while the plugin is on, and the notebooks follow the plugin's
// channel only then (ADR-0030, decision 11).
import { definePlugin } from '@mannie/plugin-sdk';
import BookOpenIcon from '@mannie/plugin-sdk/icons/book-open';
import FileTextIcon from '@mannie/plugin-sdk/icons/file-text';
import ListIcon from '@mannie/plugin-sdk/icons/list';
import SearchIcon from '@mannie/plugin-sdk/icons/search';

import NotebooksDialogs from './components/NotebooksDialogs.svelte';
import NotebooksPanel from './components/NotebooksPanel.svelte';
import NotebooksSettingsSection from './components/NotebooksSettingsSection.svelte';
import NotebooksView from './components/NotebooksView.svelte';
import { m } from './generated/messages.js';
import { notebookActions } from './state/notebook-actions.svelte.js';
import { notebookLibrary } from './state/notebooks.svelte.js';
import { NOTEBOOKS_CONTEXT } from './utils/notebooks-context.js';
import {
  notebookMentionRefusal,
  notebookMentionSummary,
  notebookMentionVariantName,
  notebooksOffQuestion,
  notebooksStartFailure,
  notebooksToolFailure,
} from './utils/notebooks-messages.js';
import { NOTEBOOKS_VIEW, SETTINGS_SECTION, shownNotebookId } from './utils/notebooks-places.js';
import {
  LIST_SOURCES_TOOL,
  listSummary,
  READ_SOURCE_TOOL,
  readSummary,
  SEARCH_TOOL,
  searchSummary,
} from './utils/notebooks-tools.js';

/** The notebook whose page the view shows, as the list knows it. */
function shownNotebook(): ReturnType<typeof notebookLibrary.find> {
  const id = shownNotebookId();
  return id === null ? undefined : notebookLibrary.find(id);
}

export default definePlugin({
  failure: notebooksStartFailure,
  offQuestion: notebooksOffQuestion,
  register(context) {
    const { slots } = context;
    notebookLibrary.follow(context);
    // The second entry of the activity bar, after Chat (D43).
    slots.navigation.register({
      icon: BookOpenIcon,
      id: NOTEBOOKS_VIEW,
      label: m.notebooks_navigation_label,
      order: 10,
      view: NOTEBOOKS_VIEW,
    });
    slots.views.register({
      component: NotebooksView,
      id: NOTEBOOKS_VIEW,
      title: () => shownNotebook()?.name ?? m.notebooks_view_title(),
      titleIsContent: () => shownNotebook() !== undefined,
    });
    slots.sidebarPanels.register({
      activity: NOTEBOOKS_VIEW,
      component: NotebooksPanel,
      id: 'notebooks.notebooks',
      order: 0,
    });
    slots.overlays.register({ component: NotebooksDialogs, id: 'notebooks.dialogs' });
    // The providers' IDs are the plugin's, so a mentioned notebook attaches
    // through the context provider (ADR-0050).
    slots.contextKinds.register(NOTEBOOKS_CONTEXT);
    slots.mentionKinds.register({
      group: m.notebooks_mention_group,
      icon: () => BookOpenIcon,
      id: context.id,
      order: 20,
      refusal: notebookMentionRefusal,
      summary: notebookMentionSummary,
      variantName: notebookMentionVariantName,
      // The provider warns about no entry.
      warning: () => null,
    });
    slots.toolKinds.register({
      failure: notebooksToolFailure,
      icon: ListIcon,
      id: LIST_SOURCES_TOOL,
      summary: listSummary,
    });
    slots.toolKinds.register({
      failure: notebooksToolFailure,
      icon: SearchIcon,
      id: SEARCH_TOOL,
      summary: searchSummary,
    });
    slots.toolKinds.register({
      failure: notebooksToolFailure,
      icon: FileTextIcon,
      id: READ_SOURCE_TOOL,
      summary: readSummary,
    });
    slots.commands.register({
      group: m.notebooks_command_group,
      id: 'notebooks.new-notebook',
      label: m.notebooks_command_new_notebook,
      order: 0,
      run: () => {
        notebookActions.startCreate();
      },
    });
    slots.settingsSections.register({
      component: NotebooksSettingsSection,
      description: m.notebooks_settings_description,
      icon: BookOpenIcon,
      id: SETTINGS_SECTION,
      order: 60,
      title: m.notebooks_settings_title,
    });
  },
});
