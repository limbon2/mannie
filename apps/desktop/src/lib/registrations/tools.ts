// The system tools feature's entries in the shell's slots (docs/features/
// system-tools/tool-activity.md; ADR-0020): the renderer of tool calls, the
// dialogs, the command for the open conversation, the full access switch in
// a conversation's toolbar (ADR-0029), the background commands' button, the
// panel of a command's output and the row of a wake-up (ADR-0041), and the
// Tools settings section. A conversation's workspace is its project's
// folder, which the chat feature shows (ADR-0023).
import SquareTerminalIcon from '@lucide/svelte/icons/square-terminal';
import WrenchIcon from '@lucide/svelte/icons/wrench';

import BackgroundCommandsControl from '#lib/components/app/tools/BackgroundCommandsControl.svelte';
import CommandNotePart from '#lib/components/app/tools/CommandNotePart.svelte';
import CommandOutputPanel from '#lib/components/app/tools/CommandOutputPanel.svelte';
import FullAccessControl from '#lib/components/app/tools/FullAccessControl.svelte';
import ToolCallPart from '#lib/components/app/tools/ToolCallPart.svelte';
import ToolsDialogs from '#lib/components/app/tools/ToolsDialogs.svelte';
import ToolsSettingsSection from '#lib/components/app/tools/ToolsSettingsSection.svelte';
import { m } from '#lib/paraglide/messages.js';
import { conversationList } from '#lib/state/conversation-list.svelte.js';
import type { ShellSlots } from '#lib/state/shell-slots.svelte.js';
import { toolsActions } from '#lib/state/tools-actions.svelte.js';
import { openConversationId } from '#lib/utils/chat-urls.js';
import { COMMAND_OUTPUT_PANEL } from '#lib/utils/tools-background.js';

/** The open conversation as the list knows it. */
function openSummary(): ReturnType<typeof conversationList.find> {
  const id = openConversationId();
  return id === null ? undefined : conversationList.find(id);
}

/** Registers the system tools feature. */
export function registerTools(slots: ShellSlots): void {
  slots.partRenderers.register({ component: ToolCallPart, id: 'tool_call' });
  slots.overlays.register({ component: ToolsDialogs, id: 'tools.dialogs' });
  slots.partRenderers.register({ component: CommandNotePart, id: 'command_note' });
  slots.conversationControls.register({ component: FullAccessControl, id: 'tools.full-access', order: 20 });
  slots.conversationControls.register({
    component: BackgroundCommandsControl,
    id: 'tools.background-commands',
    order: 15,
  });
  slots.sidePanels.register({
    component: CommandOutputPanel,
    defaultLabel: m.tools_background_panel_label,
    icon: SquareTerminalIcon,
    id: COMMAND_OUTPUT_PANEL,
  });
  slots.commands.register({
    available: () => openSummary() !== undefined,
    group: m.tools_command_group,
    id: 'tools.show-permissions',
    label: m.tools_command_show_permissions,
    order: 1,
    run: () => {
      const conversation = openSummary();
      if (conversation !== undefined) {
        toolsActions.showPermissions(conversation.id, conversation.projectId);
      }
    },
  });
  slots.settingsSections.register({
    component: ToolsSettingsSection,
    description: m.tools_settings_description,
    icon: WrenchIcon,
    id: 'tools.settings',
    order: 25,
    title: m.tools_settings_title,
  });
}
