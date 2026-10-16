// The plugins feature's entries in the shell's slots (docs/features/
// plugins.md; ADR-0030): the Plugins settings section, which lists every
// internal plugin with a switch, last among the settings.
import PlugIcon from '@lucide/svelte/icons/plug';

import PluginsSettingsSection from '#lib/components/app/plugins/PluginsSettingsSection.svelte';
import { m } from '#lib/paraglide/messages.js';
import type { ShellSlots } from '#lib/state/shell-slots.svelte.js';
import { PLUGINS_SETTINGS_SECTION } from '#lib/utils/plugins-messages.js';

/** Registers the plugins feature. */
export function registerPlugins(slots: ShellSlots): void {
  slots.settingsSections.register({
    component: PluginsSettingsSection,
    description: m.plugins_settings_description,
    icon: PlugIcon,
    id: PLUGINS_SETTINGS_SECTION,
    order: 90,
    title: m.plugins_settings_title,
  });
}
