// The UI's composition root: it creates the shell's slots and registers the
// shell's own entries, then every feature's, then every plugin's through the
// plugin's context, whose entries show only while the core reports the
// plugin as on (docs/architecture/extension-points.md, rules for every
// extension point; ADR-0030, decision 11). The plugins come from the
// generated plugin list (ADR-0035, decision 7).
import { plugins } from '#plugin-list';

import type { PluginContext } from '#lib/sdk/index.js';
import { setPluginCatalog } from '#lib/state/plugin-catalog.js';
import { type PluginStates, pluginStates } from '#lib/state/plugin-states.svelte.js';
import { ShellSlots } from '#lib/state/shell-slots.svelte.js';
import type { PluginDescriptor, PluginEntry } from '#lib/types/plugins.js';

import { type FeatureRegistration, features } from './features.js';
import { registerShell } from './shell.js';

/** What the composition root follows of the plugins' states that the core reports. */
export type PluginFollowing = Pick<PluginStates, 'isOn' | 'whileOn'>;

/** What the UI shows of a plugin outside its own code. A plugin without UI has no codes of its own. */
function describePlugin(entry: PluginEntry): PluginDescriptor {
  return {
    description: entry.description,
    failure: entry.definition?.failure ?? (() => null),
    id: entry.id,
    name: entry.name,
    offQuestion: entry.definition?.offQuestion ?? (() => null),
  };
}

/** The context in which a plugin's UI registers. */
function pluginContext(id: string, slots: ShellSlots, states: PluginFollowing): PluginContext {
  return {
    id,
    isOn: () => states.isOn(id),
    slots: slots.forPlugin(id),
    whileOn: (on, off) => states.whileOn(id, on, off),
  };
}

/**
 * The shell's slots with the shell's entries, every feature's and every
 * plugin's registered. `states` tells whether a plugin's entries show and
 * when its state turns on and off; the app follows the plugin states that
 * the core reports.
 */
export function composeShell(
  registrations: readonly FeatureRegistration[] = features,
  pluginEntries: readonly PluginEntry[] = plugins,
  states: PluginFollowing = pluginStates,
): ShellSlots {
  const slots = new ShellSlots((plugin) => states.isOn(plugin));
  registerShell(slots);
  for (const register of registrations) {
    register(slots);
  }
  setPluginCatalog(pluginEntries.map(describePlugin));
  for (const entry of pluginEntries) {
    entry.definition?.register(pluginContext(entry.id, slots, states));
  }
  return slots;
}
