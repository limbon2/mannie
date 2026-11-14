// Navigation of the TypeScript plugin SDK, `@mannie/plugin-sdk/navigation`
// (ADR-0034, decision 11; ADR-0045, decision 6): a plugin's UI opens the
// settings at one of its sections, the Plugins page, the Providers page, its
// side panels, its views and conversations, starts new conversations with
// attachments (ADR-0046, decision 6), and reads its view's parameters,
// through the shell's own functions, so the addresses stay the shell's.
import { page } from '$app/state';

import type { Attachment } from '#lib/generated/bindings.js';
import { activities, type NavigateOptions } from '#lib/state/activities.svelte.js';
import { conversationDraft } from '#lib/state/conversation-draft.svelte.js';
import { type OpenPanelOptions, type PanelLabel, sidePanels } from '#lib/state/side-panels.svelte.js';
import {
  PROVIDER_SETTINGS_SECTION,
  openConversation as showConversation,
  openConversationId as shownConversationId,
  showNewConversation,
} from '#lib/utils/chat-urls.js';
import { PLUGINS_SETTINGS_SECTION } from '#lib/utils/plugins-messages.js';
import { openSettings } from '#lib/utils/settings-sections.js';
import { openPanel } from '#lib/utils/side-panels.js';

export type { NavigateOptions } from '#lib/state/activities.svelte.js';
export type { OpenPanelOptions } from '#lib/state/side-panels.svelte.js';
export { openSettings } from '#lib/utils/settings-sections.js';

/**
 * The conversation that the chat view shows, or `null` while it shows none.
 * A new conversation is the draft of its place (ADR-0068). Read in a
 * component, it follows the address (ADR-0053, decision 8).
 */
export function openConversationId(): string | null {
  return shownConversationId();
}

/** Shows a conversation in the chat view, while the activity bar keeps its selection. */
export function openConversation(id: string): Promise<void> {
  return showConversation(id);
}

/** What a new conversation starts with. */
export interface NewConversation {
  /** What it starts with attached, with the labels that the plugin knows; the core asks each provider again. */
  readonly attachments?: readonly Attachment[];
  /**
   * The attachment of the plugin's own context provider that the
   * conversation belongs to, such as a notebook of a notebook's chat: it
   * stays attached, the conversation has no project, and the Chat activity's
   * lists leave it out, so the plugin lists it (ADR-0051).
   */
  readonly home?: Attachment;
  /** The project that it starts in, or `null` for a general conversation; a conversation with a home has none. */
  readonly projectId?: string | null;
}

/**
 * Shows the draft of a new conversation (ADR-0068) with the attachments
 * given (ADR-0046, decision 6), of the home's place (ADR-0051) or the
 * project's, while the activity bar keeps its selection.
 */
export async function startConversation(conversation: NewConversation = {}): Promise<void> {
  const home = conversation.home ?? null;
  // Before the view shows, so that it opens the draft of the home's place
  // and gives it the attachments.
  const projectId = home === null ? (conversation.projectId ?? null) : null;
  conversationDraft.reset(conversation.attachments ?? [], home, projectId);
  await showNewConversation(projectId);
}

/** Opens the Plugins page of the settings, where the owner turns plugins on and off. */
export function openPluginsPage(): Promise<void> {
  return openSettings(PLUGINS_SETTINGS_SECTION);
}

/** Opens the Providers page of the settings, where the owner adds chat and search profiles (ADR-0047). */
export function openProvidersPage(): Promise<void> {
  return openSettings(PROVIDER_SETTINGS_SECTION);
}

/** Opens a side panel of a kind that the plugin registered, beside the main view, or shows it if it is open. */
export function openSidePanel(kind: string, argument: string, options?: OpenPanelOptions): Promise<void> {
  return sidePanels.open(kind, argument, options);
}

/** Updates the tab label of an open side panel without showing it. */
export function setSidePanelLabel(kind: string, argument: string, label: PanelLabel): void {
  const { key } = openPanel(kind, argument);
  if (sidePanels.state.panels.some((panel) => panel.key === key)) {
    sidePanels.setLabel(key, label);
  }
}

/**
 * Shows a view with its parameters, such as a plugin's view at one of its
 * things, and selects the view's entry of the activity bar when it has one.
 * With `replace`, the address replaces the current one in the history, such
 * as when a view's bare address leads to one of its things.
 */
export function openView(
  view: string,
  parameters?: Readonly<Record<string, string>>,
  options?: NavigateOptions,
): Promise<void> {
  return activities.openView(view, parameters, options);
}

/** The search parameters of the main area's address, which a view reads; it follows the address in a component. */
export function viewParameters(): URLSearchParams {
  return new URLSearchParams(page.url.search);
}
