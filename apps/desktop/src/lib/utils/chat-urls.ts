// Where the chat view lives (docs/features/chat/conversation-view.md,
// layout): `?view=chat&conversation=<id>` names the open conversation, and the
// view without a conversation starts a new one.
import { goto } from '$app/navigation';
import { resolve } from '$app/paths';
import { page } from '$app/state';

import { VIEW_PARAMETER } from './views.js';

/** The ID of the chat view. */
export const CHAT_VIEW = 'chat';

/** The search parameter that names the open conversation. */
export const CONVERSATION_PARAMETER = 'conversation';

/** The ID of the provider profiles' settings section, whose page other chat views link to. */
export const PROVIDER_SETTINGS_SECTION = 'chat.providers';

/** The search parameters of a conversation in the chat view. */
export function conversationQuery(id: string): string {
  return new URLSearchParams({ [CONVERSATION_PARAMETER]: id, [VIEW_PARAMETER]: CHAT_VIEW }).toString();
}

/** The search parameters of a new conversation. */
export function newConversationQuery(): string {
  return new URLSearchParams({ [VIEW_PARAMETER]: CHAT_VIEW }).toString();
}

/** Shows a conversation. */
export async function openConversation(id: string): Promise<void> {
  // eslint-disable-next-line svelte/no-navigation-without-resolve -- the path is resolve('/'); only the search parameters change, which resolve() does not take
  await goto(`${resolve('/')}?${conversationQuery(id)}`);
}

/** Shows the view of a new conversation. */
export async function startNewConversation(): Promise<void> {
  // eslint-disable-next-line svelte/no-navigation-without-resolve -- the path is resolve('/'); only the search parameters change, which resolve() does not take
  await goto(`${resolve('/')}?${newConversationQuery()}`);
}

/** The conversation that the chat view shows, if it shows one. */
export function openConversationId(): string | null {
  const isMainRoute = page.route.id === '/';
  const view = page.url.searchParams.get(VIEW_PARAMETER);
  const isChatView = view === null || view === CHAT_VIEW;
  const showsChat = isMainRoute && isChatView;
  return showsChat ? page.url.searchParams.get(CONVERSATION_PARAMETER) : null;
}
