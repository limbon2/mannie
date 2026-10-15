// Where the chat view lives (docs/features/chat/conversation-view.md,
// layout): `?view=chat&conversation=<id>` names the open conversation, and the
// view without a conversation starts a new one, in the project that
// `?project=<id>` names (docs/features/chat/projects.md).
import { goto } from '$app/navigation';
import { resolve } from '$app/paths';
import { page } from '$app/state';

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

/** Shows the view of a new conversation, in a project or a general one. */
export async function startNewConversation(projectId: string | null = null): Promise<void> {
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

/** The conversation that the chat view shows, if it shows one. */
export function openConversationId(): string | null {
  return showsChat() ? page.url.searchParams.get(CONVERSATION_PARAMETER) : null;
}

/** The project that the pane of a new conversation starts in, if the chat view shows that pane for a project. */
export function newConversationProjectId(): string | null {
  const showsNewConversation = showsChat() && page.url.searchParams.get(CONVERSATION_PARAMETER) === null;
  return showsNewConversation ? page.url.searchParams.get(PROJECT_PARAMETER) : null;
}
