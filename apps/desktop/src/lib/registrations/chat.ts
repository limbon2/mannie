// The chat feature's entries in the shell's slots (docs/features/chat/
// conversation-view.md; ADR-0020): the conversation view, which Mannie opens
// into, the conversation list, the part renderers, the dialogs, the commands
// and the settings sections.
import ScrollTextIcon from '@lucide/svelte/icons/scroll-text';
import ServerIcon from '@lucide/svelte/icons/server';

import ChatDialogs from '#lib/components/app/chat/ChatDialogs.svelte';
import ConversationListPanel from '#lib/components/app/chat/ConversationListPanel.svelte';
import ConversationView from '#lib/components/app/chat/ConversationView.svelte';
import ErrorPart from '#lib/components/app/chat/ErrorPart.svelte';
import InstructionsSettingsSection from '#lib/components/app/chat/InstructionsSettingsSection.svelte';
import ProviderSettingsSection from '#lib/components/app/chat/ProviderSettingsSection.svelte';
import ReasoningPart from '#lib/components/app/chat/ReasoningPart.svelte';
import TextPart from '#lib/components/app/chat/TextPart.svelte';
import UserTextPart from '#lib/components/app/chat/UserTextPart.svelte';
import { m } from '#lib/paraglide/messages.js';
import { chatActions } from '#lib/state/chat-actions.svelte.js';
import { conversationList } from '#lib/state/conversation-list.svelte.js';
import type { ShellSlots } from '#lib/state/shell-slots.svelte.js';
import {
  CHAT_VIEW,
  openConversationId,
  PROVIDER_SETTINGS_SECTION,
  startNewConversation,
} from '#lib/utils/chat-urls.js';

/** The open conversation as the list knows it. */
function openSummary(): ReturnType<typeof conversationList.find> {
  const id = openConversationId();
  return id === null ? undefined : conversationList.find(id);
}

/** Registers the chat feature. */
export function registerChat(slots: ShellSlots): void {
  slots.views.register({
    component: ConversationView,
    id: CHAT_VIEW,
    title: () => openSummary()?.title ?? m.chat_view_title(),
    titleIsContent: () => openSummary() !== undefined,
  });
  slots.sidebarPanels.register({ component: ConversationListPanel, id: 'chat.conversations', order: 0 });
  slots.overlays.register({ component: ChatDialogs, id: 'chat.dialogs' });
  slots.partRenderers.register({ component: UserTextPart, id: 'user_text' });
  slots.partRenderers.register({ component: TextPart, id: 'text' });
  slots.partRenderers.register({ component: ReasoningPart, id: 'reasoning' });
  slots.partRenderers.register({ component: ErrorPart, id: 'error' });
  slots.commands.register({
    group: m.chat_command_group,
    id: 'chat.new-conversation',
    label: m.chat_new_conversation,
    order: 0,
    run: () => void startNewConversation(),
  });
  slots.commands.register({
    available: () => openSummary() !== undefined,
    group: m.chat_command_group,
    id: 'chat.rename-conversation',
    label: m.chat_command_rename,
    order: 1,
    run: () => {
      const conversation = openSummary();
      if (conversation !== undefined) {
        chatActions.startRename(conversation);
      }
    },
  });
  slots.commands.register({
    available: () => openSummary() !== undefined,
    group: m.chat_command_group,
    id: 'chat.compact-conversation',
    label: m.chat_command_compact,
    order: 2,
    run: () => {
      const id = openConversationId();
      if (id !== null) {
        void chatActions.compact(id);
      }
    },
  });
  slots.settingsSections.register({
    component: ProviderSettingsSection,
    description: m.chat_providers_section_description,
    icon: ServerIcon,
    id: PROVIDER_SETTINGS_SECTION,
    order: 10,
    title: m.chat_providers_section_title,
  });
  slots.settingsSections.register({
    component: InstructionsSettingsSection,
    description: m.chat_instructions_section_description,
    icon: ScrollTextIcon,
    id: 'chat.instructions',
    order: 20,
    title: m.chat_instructions_section_title,
  });
}
