// The chats of a notebook (plugins/notebooks/docs/README.md, chats;
// ADR-0051): the conversations that belong to it, which the plugin lists
// under the notebook instead of the Chat activity, and the other chats that
// attached it with "@" (ADR-0050), as the core's list of conversations knows
// them, the most recently changed first.
import { type AttachmentRef, type ConversationSummary, listConversations } from '@mannie/plugin-sdk/conversations';

import { PLUGIN_ID } from '../generated/plugin.js';

/** Whether an attachment names the notebook. */
export function namesNotebook(attachment: AttachmentRef, notebookId: string): boolean {
  const isPlugins = attachment.provider === PLUGIN_ID;
  const isNotebook = attachment.key === notebookId;
  return isPlugins && isNotebook;
}

/** Whether a conversation is one of the notebook's chats, which belong to it. */
export function isChatOf(conversation: ConversationSummary, notebookId: string): boolean {
  const home = conversation.home ?? null;
  if (home === null) {
    return false;
  }
  return namesNotebook(home, notebookId);
}

/** Whether a conversation has the notebook attached, as its own chats and the chats that named it do. */
export function usesNotebook(conversation: ConversationSummary, notebookId: string): boolean {
  const attachments = conversation.attachments ?? [];
  return attachments.some((attachment) => namesNotebook(attachment, notebookId));
}

/** The notebook's own chats; read in a component, they follow the list. */
export function chatsOf(notebookId: string): ConversationSummary[] {
  return listConversations().filter((conversation) => isChatOf(conversation, notebookId));
}

/** The other chats that have the notebook attached; read in a component, they follow the list. */
export function chatsUsing(notebookId: string): ConversationSummary[] {
  return listConversations().filter(
    (conversation) => !isChatOf(conversation, notebookId) && usesNotebook(conversation, notebookId),
  );
}
