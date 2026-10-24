// Conversations of the TypeScript plugin SDK, `@mannie/plugin-sdk/conversations`
// (ADR-0046, decision 6; ADR-0051): a plugin's UI reads the owner's
// conversations with their attachments and homes, attaches its context
// provider's keys to them and detaches them, through the core's commands,
// which ask the provider again, and deletes the conversations of its items.
// The core words the failures of its commands (ADR-0019), and the plugin's
// kind of context words the refusals of its keys.
import {
  type AttachmentRef,
  type ChatError,
  type CommandError,
  commands,
  type ConversationSummary,
} from '#lib/generated/bindings.js';
import { conversationList } from '#lib/state/conversation-list.svelte.js';
import type { Failure, Outcome } from '#lib/types/calls.js';
import type { ContextKind } from '#lib/types/shell-slots.js';
import { call } from '#lib/utils/calls.js';
import { chatFailureMessage } from '#lib/utils/chat-errors.js';

export type {
  Attachment,
  AttachmentRef,
  ChatError,
  CommandError,
  ConversationSummary,
} from '#lib/generated/bindings.js';

/** Why a conversation's command failed, or that the core did not answer. */
export type ConversationOutcome = Outcome<null, CommandError<ChatError>>;

/** Why a conversation's command failed. */
export type ConversationFailure = Failure<CommandError<ChatError>>;

/**
 * Starts following the owner's conversations, which `listConversations`
 * then lists; a component that lists them calls it as it mounts. Later calls
 * change nothing.
 */
export function followConversations(): void {
  conversationList.start();
}

/**
 * The owner's conversations, the most recently changed first, as the window's
 * conversation list knows them; read in a component, it follows the list.
 * Task conversations never show in it. The conversations that belong to a
 * plugin's item carry it as their `home`, and the Chat activity's lists leave
 * them out, so the plugin lists them (ADR-0051).
 */
export function listConversations(): readonly ConversationSummary[] {
  return conversationList.conversations;
}

/**
 * What to tell the owner about a conversation's command that failed. The
 * plugin's kind of context words the refusals of its own keys.
 */
export function conversationFailureMessage(failure: ConversationFailure, kind?: ContextKind): string {
  return chatFailureMessage(failure, undefined, undefined, (provider) => (provider === kind?.id ? kind : undefined));
}

/**
 * Whether the conversation list was read: `loading` until then, and `error`
 * when it could not be; meanwhile it lists none.
 */
export function conversationsStatus(): 'error' | 'loading' | 'ready' {
  return conversationList.status;
}

/** Follows the conversation list again after it could not be read. */
export function retryConversations(): void {
  conversationList.retry();
}

/** Attaches one of the plugin's keys to a conversation, once its context provider described it. */
export function attach(conversationId: string, attachment: AttachmentRef): Promise<ConversationOutcome> {
  return call(() => commands.attachContext({ attachment, conversationId }));
}

/** Detaches a key from a conversation. The core refuses to detach the conversation's home. */
export function detach(conversationId: string, attachment: AttachmentRef): Promise<ConversationOutcome> {
  return call(() => commands.detachContext({ attachment, conversationId }));
}

/**
 * Deletes a conversation with its replies, such as a chat of an item of the
 * plugin's that the owner deletes with the item (ADR-0051, decision 6).
 */
export function deleteConversation(conversationId: string): Promise<ConversationOutcome> {
  return call(() => commands.deleteConversation({ conversationId }));
}
