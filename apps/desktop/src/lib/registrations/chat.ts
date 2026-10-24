// The chat feature's entries in the shell's slots (docs/features/chat/
// conversation-view.md, projects.md; ADR-0020, ADR-0025): the conversation
// view, which Mannie opens into, the projects and the conversation list in the
// sidebar, the side panel that shows a conversation, the part renderers, the
// dialogs, the commands and the settings sections.
import MessagesSquareIcon from '@lucide/svelte/icons/messages-square';
import ScrollTextIcon from '@lucide/svelte/icons/scroll-text';
import ServerIcon from '@lucide/svelte/icons/server';

import ChatDialogs from '#lib/components/app/chat/ChatDialogs.svelte';
import ContextPart from '#lib/components/app/chat/ContextPart.svelte';
import ConversationListPanel from '#lib/components/app/chat/ConversationListPanel.svelte';
import ConversationPanel from '#lib/components/app/chat/ConversationPanel.svelte';
import ConversationView from '#lib/components/app/chat/ConversationView.svelte';
import ErrorPart from '#lib/components/app/chat/ErrorPart.svelte';
import ImagePart from '#lib/components/app/chat/ImagePart.svelte';
import InstructionsSettingsSection from '#lib/components/app/chat/InstructionsSettingsSection.svelte';
import MentionPart from '#lib/components/app/chat/MentionPart.svelte';
import ProjectsPanel from '#lib/components/app/chat/ProjectsPanel.svelte';
import ProviderSettingsSection from '#lib/components/app/chat/ProviderSettingsSection.svelte';
import ReasoningPart from '#lib/components/app/chat/ReasoningPart.svelte';
import TextPart from '#lib/components/app/chat/TextPart.svelte';
import UserTextPart from '#lib/components/app/chat/UserTextPart.svelte';
import { m } from '#lib/paraglide/messages.js';
import { chatActions } from '#lib/state/chat-actions.svelte.js';
import { conversationList } from '#lib/state/conversation-list.svelte.js';
import { projectActions } from '#lib/state/project-actions.svelte.js';
import type { ShellSlots } from '#lib/state/shell-slots.svelte.js';
import {
  CHAT_VIEW,
  CONVERSATION_PANEL,
  openConversationId,
  PROVIDER_SETTINGS_SECTION,
  startNewConversation,
} from '#lib/utils/chat-urls.js';
import {
  fileMentionIcon,
  fileMentionRefusal,
  fileMentionSummary,
  fileMentionVariantName,
  fileMentionWarning,
  FILES_PROVIDER,
} from '#lib/utils/file-mentions.js';

/** The open conversation as the list knows it. */
function openSummary(): ReturnType<typeof conversationList.find> {
  const id = openConversationId();
  return id === null ? undefined : conversationList.find(id);
}

/** Registers the chat feature. */
export function registerChat(slots: ShellSlots): void {
  // The first entry of the activity bar, so the sidebar panels that name no
  // entry, the projects and the conversations among them, are chat's
  // (ADR-0045, decision 2).
  slots.navigation.register({
    icon: MessagesSquareIcon,
    id: CHAT_VIEW,
    label: m.chat_navigation_label,
    order: 0,
    view: CHAT_VIEW,
  });
  slots.views.register({
    component: ConversationView,
    id: CHAT_VIEW,
    title: () => openSummary()?.title ?? m.chat_view_title(),
    titleIsContent: () => openSummary() !== undefined,
  });
  slots.sidebarPanels.register({ component: ProjectsPanel, id: 'chat.projects', order: 0 });
  slots.sidebarPanels.register({ component: ConversationListPanel, id: 'chat.conversations', order: 10 });
  slots.sidePanels.register({
    component: ConversationPanel,
    defaultLabel: m.chat_panel_label,
    icon: MessagesSquareIcon,
    id: CONVERSATION_PANEL,
  });
  slots.overlays.register({ component: ChatDialogs, id: 'chat.dialogs' });
  slots.partRenderers.register({ component: UserTextPart, id: 'user_text' });
  slots.partRenderers.register({ component: TextPart, id: 'text' });
  slots.partRenderers.register({ component: ReasoningPart, id: 'reasoning' });
  slots.partRenderers.register({ component: ErrorPart, id: 'error' });
  slots.partRenderers.register({ component: MentionPart, id: 'mention' });
  slots.partRenderers.register({ component: ImagePart, id: 'image' });
  slots.partRenderers.register({ component: ContextPart, id: 'context' });
  slots.mentionKinds.register({
    group: m.chat_mention_files_group,
    icon: fileMentionIcon,
    id: FILES_PROVIDER,
    order: 0,
    refusal: fileMentionRefusal,
    summary: fileMentionSummary,
    variantName: fileMentionVariantName,
    warning: fileMentionWarning,
  });
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
  slots.commands.register({
    group: m.chat_command_group,
    id: 'chat.add-project',
    label: m.chat_command_add_project,
    order: 3,
    run: () => void projectActions.add(),
  });
  slots.commands.register({
    group: m.chat_command_group,
    id: 'chat.add-project-from-path',
    label: m.chat_command_add_project_from_path,
    order: 4,
    run: () => projectActions.startAddFromPath(),
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
