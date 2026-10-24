// The MCP plugin's UI (plugins/mcp/docs/README.md; ADR-0043): the MCP
// servers section of the settings with its dialogs, "Add MCP server…" in the
// command palette, the kind of mentions that attaches a server's resource
// after "@", and the cards of the servers' tools, which one entry describes,
// since the plugin publishes them while it runs (ADR-0043, decision 9). Every
// entry shows only while the plugin is on, and the list of servers follows
// the plugin's channel only then (ADR-0030, decision 11).
import { definePlugin } from '@mannie/plugin-sdk';
import FileTextIcon from '@mannie/plugin-sdk/icons/file-text';
import PlugIcon from '@mannie/plugin-sdk/icons/plug';
import { openSettings } from '@mannie/plugin-sdk/navigation';

import McpDialogs from './components/McpDialogs.svelte';
import McpSettingsSection from './components/McpSettingsSection.svelte';
import { m } from './generated/messages.js';
import { mcpActions, SETTINGS_SECTION } from './state/mcp-actions.svelte.js';
import { serverList } from './state/servers.svelte.js';
import {
  mcpOffQuestion,
  mcpStartFailureMessage,
  mcpToolFailure,
  resourceMentionRefusal,
  resourceMentionSummary,
  resourceMentionVariantName,
} from './utils/mcp-messages.js';

export default definePlugin({
  failure: mcpStartFailureMessage,
  offQuestion: mcpOffQuestion,
  register(context) {
    const { slots } = context;
    serverList.follow(context);
    slots.overlays.register({ component: McpDialogs, id: 'mcp.dialogs' });
    slots.commands.register({
      group: m.mcp_command_group,
      id: 'mcp.add-server',
      label: m.mcp_command_add_server,
      order: 0,
      run: () => {
        void openSettings(SETTINGS_SECTION).then(() => {
          mcpActions.startAdd();
        });
      },
    });
    slots.settingsSections.register({
      component: McpSettingsSection,
      description: m.mcp_settings_description,
      icon: PlugIcon,
      id: SETTINGS_SECTION,
      order: 50,
      title: m.mcp_settings_title,
    });
    // The core names a call by its external-tool access; the plugin gives
    // the icon and words its failure codes, for every tool that it publishes.
    slots.toolKinds.register({
      everyTool: true,
      failure: mcpToolFailure,
      icon: PlugIcon,
      id: 'mcp.servers',
      summary: () => null,
    });
    // The provider's ID is the plugin's.
    slots.mentionKinds.register({
      group: m.mcp_mention_group,
      icon: () => FileTextIcon,
      id: context.id,
      order: 30,
      refusal: resourceMentionRefusal,
      summary: resourceMentionSummary,
      variantName: resourceMentionVariantName,
      warning: () => null,
    });
  },
});
