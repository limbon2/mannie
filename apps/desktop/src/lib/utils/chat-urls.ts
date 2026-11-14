// Where the chat view lives (docs/features/chat/conversation-view.md,
// layout): `?view=chat&conversation=<id>` names the open conversation, and the
// view without a conversation shows the draft of a new one (ADR-0068), in the
// project that `?project=<id>` names (docs/features/chat/projects.md).
import { goto } from '$app/navigation';
import { resolve } from '$app/paths';
import { page } from '$app/state';

import { conversationDraft } from '#lib/state/conversation-draft.svelte.js';
import { shownDraft } from '#lib/state/shown-draft.svelte.js';

import { VIEW_PARAMETER } from './views.js';

/** The ID of the chat view. */
export const CHAT_VIEW = 'chat';

/** The search parameter that names the open conversation. */
export const CONVERSATION_PARAMETER = 'conversation';

/** The search parameter that names the project of a new conversation. */
export const PROJECT_PARAMETER = 'project';

/** The ID of the provider profiles' settings section, whose page other chat views link to. */
export const PROVIDER_SETTINGS_SECTION = 'chat.providers';

/**
 * The kind of side panel that shows a conversation read-only, with the
 * conversation's ID as its argument (ADR-0025, decision 7). Other features
 * open it by this ID, such as child agents for their task conversations.
 */
export const CONVERSATION_PANEL = 'chat.conversation';

/** The search parameters of a conversation in the chat view. */
export function conversationQuery(id: string): string {
  return new URLSearchParams({ [CONVERSATION_PARAMETER]: id, [VIEW_PARAMETER]: CHAT_VIEW }).toString();
}

/** The search parameters of a new conversation, in a project or a general one. */
export function newConversationQuery(projectId: string | null = null): string {
  if (projectId === null) {
    return new URLSearchParams({ [VIEW_PARAMETER]: CHAT_VIEW }).toString();
  }
  return new URLSearchParams({ [PROJECT_PARAMETER]: projectId, [VIEW_PARAMETER]: CHAT_VIEW }).toString();
}

/** Shows a conversation. */
export async function openConversation(id: string): Promise<void> {
  // eslint-disable-next-line svelte/no-navigation-without-resolve -- the path is resolve('/'); only the search parameters change, which resolve() does not take
  await goto(`${resolve('/')}?${conversationQuery(id)}`);
}

/**
 * Names the draft that the view shows in the address, once its first message
 * made it a conversation of the list (ADR-0068). The pane stays, so the
 * address keeps its side panels, such as the draft's terminal, and the focus
 * stays in the message box.
 */
export async function nameShownDraft(id: string): Promise<void> {
  const search = new URLSearchParams(page.url.search);
  search.set(CONVERSATION_PARAMETER, id);
  search.delete(PROJECT_PARAMETER);
  // eslint-disable-next-line svelte/no-navigation-without-resolve -- the path is resolve('/'); only the search parameters change, which resolve() does not take
  await goto(`${resolve('/')}?${search.toString()}`, { replace: true, reset: false });
}

/**
 * Shows the view of a new conversation, in a project or a general one. It is
 * one of the Chat's, so the draft lets go of a home that a plugin gave it
 * (ADR-0051).
 */
export async function startNewConversation(projectId: string | null = null): Promise<void> {
  conversationDraft.leaveHome();
  await showNewConversation(projectId);
}

/** Shows the view of a new conversation, of the home that the draft holds, if any. */
export async function showNewConversation(projectId: string | null = null): Promise<void> {
  // eslint-disable-next-line svelte/no-navigation-without-resolve -- the path is resolve('/'); only the search parameters change, which resolve() does not take
  await goto(`${resolve('/')}?${newConversationQuery(projectId)}`);
}

/** Whether the page shows the chat view. */
function showsChat(): boolean {
  const isMainRoute = page.route.id === '/';
  const view = page.url.searchParams.get(VIEW_PARAMETER);
  const isChatView = view === null || view === CHAT_VIEW;
  return isMainRoute && isChatView;
}

/**
 * The conversation that the chat view shows, if it shows one: the one that
 * the address names, or the draft of a new one (ADR-0068).
 */
export function openConversationId(): string | null {
  if (!showsChat()) {
    return null;
  }
  return page.url.searchParams.get(CONVERSATION_PARAMETER) ?? shownDraft.id;
}

/** The project that the pane of a new conversation starts in, if the chat view shows that pane for a project. */
export function newConversationProjectId(): string | null {
  const showsNewConversation = showsChat() && page.url.searchParams.get(CONVERSATION_PARAMETER) === null;
  return showsNewConversation ? page.url.searchParams.get(PROJECT_PARAMETER) : null;
}
