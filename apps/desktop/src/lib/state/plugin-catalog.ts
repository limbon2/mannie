// The plugins that this version of Mannie lists, as the UI names them
// (ADR-0030, decision 3). The composition root fills it once from its list,
// so that the core's messages and the Plugins page can name a plugin without
// importing the plugin's code (ADR-0030, decision 10).
import type { PluginDescriptor } from '#lib/types/plugins.js';

let catalog: readonly PluginDescriptor[] = [];

/** Replaces the catalog with the composition root's list. */
export function setPluginCatalog(plugins: readonly PluginDescriptor[]): void {
  catalog = plugins;
}

/** The plugins, in the composition root's order. */
export function pluginCatalog(): readonly PluginDescriptor[] {
  return catalog;
}

/** The plugin with this ID, if this version lists it. */
export function findPlugin(id: string): PluginDescriptor | undefined {
  return catalog.find((plugin) => plugin.id === id);
}

/** A plugin's name, or its ID for a plugin that this version does not list. */
export function pluginName(id: string): string {
  return findPlugin(id)?.name() ?? id;
}
