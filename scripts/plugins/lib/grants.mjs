// @ts-check
// What each plugin may use, for the review of a plugin's change (ADR-0034,
// decision 13): the services that its manifest declares, the sensitive ones
// marked, its entries in the credential store, the commands and channels of
// its interface, its programs, the crates that only it may use and the
// direct native calls that its manifest grants (ADR-0035, decision 6). The
// Plugins page shows the same for each plugin. `task plugins:grants` prints
// it.

import { nativeGrants } from './native.mjs';

/**
 * A plugin's interface, as `task gen` writes it to
 * `.generated/desktop/interfaces.json`.
 * @typedef {object} PluginInterface
 * @property {string[]} channels
 * @property {string[]} commands
 * @property {string} id
 */

/**
 * A list for a line: its items joined, or "none".
 * @param {string[]} items
 * @param {string} [separator]
 */
function listOf(items, separator = ', ') {
  return items.length > 0 ? items.join(separator) : 'none';
}

/**
 * What a plugin may use, as lines of text: a line with its ID and folder,
 * then one line for each kind of grant.
 * @param {import('./manifests.mjs').PluginFolder} plugin
 * @param {PluginInterface} pluginInterface
 * @param {string[]} sensitiveServices the services that reach sensitive data
 * @returns {string[]}
 */
export function grantLines(plugin, pluginInterface, sensitiveServices) {
  const { manifest } = plugin;
  const services = Object.entries(manifest.services ?? {}).map(([name, major]) => {
    const mark = sensitiveServices.includes(name) ? ' (sensitive)' : '';
    return `${name} ${String(major)}${mark}`;
  });
  const secrets = [
    ...(manifest.secrets?.entries ?? []).map((entry) => `${plugin.id}/${entry}`),
    ...(manifest.secrets?.legacy ?? []).map((name) => `${name} (a legacy name)`),
    ...(manifest.secrets?.keyed ?? []).map((family) => `${plugin.id}/${family}/<key> (a keyed family)`),
  ];
  const programs = (manifest.programs ?? []).map((program) => {
    const crates = program.crates ?? [];
    return crates.length > 0 ? `${program.package}, with ${crates.join(', ')}` : program.package;
  });
  const native = nativeGrants(manifest).map((grant) => `    ${grant.category}: ${grant.reason}`);
  const where = plugin.validation ? `${plugin.folder}, in validation builds only` : plugin.folder;
  return [
    `${plugin.id} (${where})`,
    `  Services: ${listOf(services)}`,
    `  Secret entries: ${listOf(secrets)}`,
    `  Commands: ${listOf(pluginInterface.commands)}`,
    `  Channels: ${listOf(pluginInterface.channels)}`,
    `  Programs: ${listOf(programs, '; ')}`,
    `  Crates that only it may use: ${listOf(manifest.crates?.exclusive ?? [])}`,
    ...(native.length > 0 ? ['  Native calls:', ...native] : ['  Native calls: none']),
  ];
}
