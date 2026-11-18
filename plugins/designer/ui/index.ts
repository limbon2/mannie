// The Designer plugin's UI (plugins/designer/docs/README.md): its entry of the
// activity bar, main view, project sidebar and create dialog. The shell shows
// these entries and the project list follows the channel while the plugin is
// on (ADR-0030, decision 11).
import { definePlugin } from '@mannie/plugin-sdk';
import PenToolIcon from '@mannie/plugin-sdk/icons/pen-tool';

import DesignerDialogs from './components/DesignerDialogs.svelte';
import DesignerPanel from './components/DesignerPanel.svelte';
import DesignerView from './components/DesignerView.svelte';
import { m } from './generated/messages.js';
import { projectLibrary } from './state/projects.svelte.js';

const DESIGNER_VIEW = 'designer';

export default definePlugin({
  register(context) {
    const { slots } = context;
    projectLibrary.follow(context);
    slots.navigation.register({
      // After Chat, Agents and Notebooks, and before the Databases of Workers.
      icon: PenToolIcon,
      id: DESIGNER_VIEW,
      label: m.designer_navigation_label,
      order: 15,
      view: DESIGNER_VIEW,
    });
    slots.views.register({
      component: DesignerView,
      id: DESIGNER_VIEW,
      title: m.designer_view_title,
    });
    slots.sidebarPanels.register({
      activity: DESIGNER_VIEW,
      component: DesignerPanel,
      id: 'designer.projects',
      order: 0,
    });
    slots.overlays.register({ component: DesignerDialogs, id: 'designer.dialogs' });
  },
});
