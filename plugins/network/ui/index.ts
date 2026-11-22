// The Network Client plugin's UI (plugins/network/docs/README.md; ADR-0075):
// the Network activity bar entry, its Postman-style workbench view, the
// collections sidebar tree, modal overlays, `@` mention kind, custom tool cards
// and command palette entries. Every entry shows only while the plugin is on.
import { definePlugin } from '@mannie/plugin-sdk';
import FileCodeIcon from '@mannie/plugin-sdk/icons/file-code';
import GlobeIcon from '@mannie/plugin-sdk/icons/globe';
import ListIcon from '@mannie/plugin-sdk/icons/list';
import SendIcon from '@mannie/plugin-sdk/icons/send';

import NetworkDialogs from './components/NetworkDialogs.svelte';
import NetworkPanel from './components/NetworkPanel.svelte';
import NetworkToolDetails from './components/NetworkToolDetails.svelte';
import NetworkView from './components/NetworkView.svelte';
import { m } from './generated/messages.js';
import { networkDialogs } from './state/network-dialogs.svelte.js';
import { networkLibrary } from './state/network.svelte.js';
import {
  networkMentionRefusal,
  networkMentionSummary,
  networkMentionVariantName,
  networkOffQuestion,
  networkStartFailure,
  networkToolFailure,
} from './utils/network-messages.js';
import { NETWORK_VIEW, openNetwork, shownCollectionId, shownRequestId } from './utils/network-places.js';
import {
  GET_REQUEST_TOOL,
  getRequestSummary,
  LIST_COLLECTIONS_TOOL,
  LIST_REQUESTS_TOOL,
  listCollectionsSummary,
  listRequestsSummary,
  SEND_REQUEST_TOOL,
  sendRequestSummary,
} from './utils/network-tools.js';

function activeTitle(): string {
  const reqId = shownRequestId();
  const colId = shownCollectionId();
  if (reqId !== null) {
    const found = networkLibrary.findRequest(reqId, colId ?? undefined);
    if (found !== undefined) {
      return `${found.request.method} ${found.request.name}`;
    }
  }
  if (colId !== null) {
    const col = networkLibrary.findCollection(colId);
    if (col !== undefined) {
      return col.name;
    }
  }
  return m.network_view_title();
}

function titleShowsContent(): boolean {
  const reqId = shownRequestId();
  const colId = shownCollectionId();
  if (reqId !== null) {
    return networkLibrary.findRequest(reqId, colId ?? undefined) !== undefined;
  }
  if (colId !== null) {
    return networkLibrary.findCollection(colId) !== undefined;
  }
  return false;
}

export default definePlugin({
  failure: networkStartFailure,
  offQuestion: networkOffQuestion,
  register(context) {
    const { slots } = context;
    networkLibrary.follow(context);

    slots.navigation.register({
      icon: GlobeIcon,
      id: NETWORK_VIEW,
      label: m.network_navigation_label,
      order: 25,
      sidebarResizable: true,
      view: NETWORK_VIEW,
    });
    slots.views.register({
      component: NetworkView,
      id: NETWORK_VIEW,
      title: activeTitle,
      titleIsContent: titleShowsContent,
    });
    slots.sidebarPanels.register({
      activity: NETWORK_VIEW,
      component: NetworkPanel,
      id: 'network.collections',
      order: 0,
    });
    slots.overlays.register({
      component: NetworkDialogs,
      id: 'network.dialogs',
    });
    slots.mentionKinds.register({
      group: m.network_mention_group,
      icon: (variant) => (variant === 'request' ? SendIcon : GlobeIcon),
      id: context.id,
      order: 40,
      refusal: networkMentionRefusal,
      summary: networkMentionSummary,
      variantName: networkMentionVariantName,
      warning: () => null,
    });
    slots.toolKinds.register({
      failure: networkToolFailure,
      icon: ListIcon,
      id: LIST_COLLECTIONS_TOOL,
      summary: listCollectionsSummary,
    });
    slots.toolKinds.register({
      failure: networkToolFailure,
      icon: ListIcon,
      id: LIST_REQUESTS_TOOL,
      summary: listRequestsSummary,
    });
    slots.toolKinds.register({
      failure: networkToolFailure,
      icon: FileCodeIcon,
      id: GET_REQUEST_TOOL,
      summary: getRequestSummary,
    });
    slots.toolKinds.register({
      details: NetworkToolDetails,
      failure: networkToolFailure,
      icon: SendIcon,
      id: SEND_REQUEST_TOOL,
      summary: sendRequestSummary,
    });
    slots.commands.register({
      group: m.network_command_group,
      id: 'network.new-collection',
      label: m.network_command_new_collection,
      order: 0,
      run: () => {
        void openNetwork().then(() => {
          networkDialogs.startCreateCollection();
        });
      },
    });
    slots.commands.register({
      group: m.network_command_group,
      id: 'network.import-openapi',
      label: m.network_command_import_openapi,
      order: 1,
      run: () => {
        void openNetwork().then(() => {
          networkDialogs.openImport();
        });
      },
    });
    slots.commands.register({
      group: m.network_command_group,
      id: 'network.cookies',
      label: m.network_command_open_cookies,
      order: 2,
      run: () => {
        networkDialogs.openCookieJar();
      },
    });
    slots.commands.register({
      group: m.network_command_group,
      id: 'network.global-variables',
      label: m.network_command_open_globals,
      order: 3,
      run: () => {
        networkDialogs.openGlobalVariables();
      },
    });
  },
});
