// The UI's composition root: it creates the shell's slots and registers the
// shell's own entries, then every feature's, then every plugin's through the
// plugin's own view of the slots, whose entries show only while the core
// reports the plugin as on (docs/architecture/extension-points.md, rules for
// every extension point; ADR-0030, decision 11).
import { setPluginCatalog } from '#lib/state/plugin-catalog.js';
import { pluginStates } from '#lib/state/plugin-states.svelte.js';
import { type PluginIsOn, ShellSlots } from '#lib/state/shell-slots.svelte.js';
import type { PluginRegistration } from '#lib/types/plugins.js';

import { type FeatureRegistration, features } from './features.js';
import { plugins } from './plugin-list.js';
import { registerShell } from './shell.js';

/**
 * The shell's slots with the shell's entries, every feature's and every
 * plugin's registered. `isOn` tells whether a plugin's entries show; the app
 * asks the plugin states that the core reports.
 */
export function composeShell(
  registrations: readonly FeatureRegistration[] = features,
  pluginRegistrations: readonly PluginRegistration[] = plugins,
  isOn: PluginIsOn = (plugin) => pluginStates.isOn(plugin),
): ShellSlots {
  const slots = new ShellSlots(isOn);
  registerShell(slots);
  for (const register of registrations) {
    register(slots);
  }
  setPluginCatalog(pluginRegistrations);
  for (const plugin of pluginRegistrations) {
    plugin.register(slots.forPlugin(plugin.id));
  }
  return slots;
}
