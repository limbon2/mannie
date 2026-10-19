// Navigation of the TypeScript plugin SDK, `@mannie/plugin-sdk/navigation`
// (ADR-0034, decision 11): a plugin's UI opens the settings at one of its
// sections, the Plugins page and its side panels through the shell's own
// functions, so the addresses stay the shell's.
import { type OpenPanelOptions, sidePanels } from '#lib/state/side-panels.svelte.js';
import { PLUGINS_SETTINGS_SECTION } from '#lib/utils/plugins-messages.js';
import { openSettings } from '#lib/utils/settings-sections.js';

export type { OpenPanelOptions } from '#lib/state/side-panels.svelte.js';
export { openSettings } from '#lib/utils/settings-sections.js';

/** Opens the Plugins page of the settings, where the owner turns plugins on and off. */
export function openPluginsPage(): Promise<void> {
  return openSettings(PLUGINS_SETTINGS_SECTION);
}

/** Opens a side panel of a kind that the plugin registered, beside the main view, or shows it if it is open. */
export function openSidePanel(kind: string, argument: string, options?: OpenPanelOptions): Promise<void> {
  return sidePanels.open(kind, argument, options);
}
