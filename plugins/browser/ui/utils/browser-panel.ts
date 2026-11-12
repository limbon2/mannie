import { call } from '@mannie/plugin-sdk/ipc';
import {
  openConversationId,
  type OpenPanelOptions,
  openSidePanel,
  setSidePanelLabel,
} from '@mannie/plugin-sdk/navigation';

import { type BrowserStatus, type BrowserTab, commands } from '../generated/bindings.js';
import { m } from '../generated/messages.js';

export const BROWSER_PANEL = 'browser.session';

export function browserPanelTarget(argument: string): { conversationId: string; tabId: string | null } {
  const separator = argument.lastIndexOf('.');
  const addressesTab = separator > 0 && separator < argument.length - 1;
  return addressesTab
    ? { conversationId: argument.slice(0, separator), tabId: argument.slice(separator + 1) }
    : { conversationId: argument, tabId: null };
}

export function browserTabLabel(tab: BrowserTab): { isContent: boolean; text: string } {
  const blank = tab.title === '' && (tab.url === '' || tab.url === 'about:blank');
  return blank
    ? { isContent: false, text: m.browser_blank_tab() }
    : { isContent: true, text: tab.title === '' ? tab.url : tab.title };
}

export async function readBrowserStatus(conversationId: string): Promise<BrowserStatus | null> {
  const result = await call(() => commands.browserStatus({ conversationId }));
  return result.ok ? result.data : null;
}

function tabArgument(conversationId: string, tab: BrowserTab): string {
  return `${conversationId}.${tab.id}`;
}

export function labelBrowserTab(conversationId: string, tab: BrowserTab): void {
  setSidePanelLabel(BROWSER_PANEL, tabArgument(conversationId, tab), browserTabLabel(tab));
}

export async function openBrowserTab(
  conversationId: string,
  tab: BrowserTab,
  options: OpenPanelOptions = {},
): Promise<void> {
  if (openConversationId() === conversationId) {
    await openSidePanel(BROWSER_PANEL, tabArgument(conversationId, tab), { label: browserTabLabel(tab), ...options });
  }
}

export async function openBrowser(conversationId: string, opener: HTMLElement | null): Promise<void> {
  const status = await readBrowserStatus(conversationId);
  if (openConversationId() !== conversationId) {
    return;
  }
  const active = status?.tabs.find((tab) => tab.id === status.activeTab);
  if (active === undefined) {
    await openSidePanel(BROWSER_PANEL, conversationId, { opener });
    return;
  }
  await openBrowserTab(conversationId, active, { opener, replaceArgument: conversationId });
}
