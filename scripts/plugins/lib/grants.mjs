// @ts-check
// What each plugin may use, for the review of a plugin's change (ADR-0034,
// decision 13): the services that its manifest declares, the sensitive ones
// marked, its entries in the credential store, the commands and channels of
// its interface, its programs, the installed programs that it runs with
// their reasons, fixed arguments and inherited variables (ADR-0038, decision
// 7), the crates that only it may use, the direct native calls that its
// manifest grants (ADR-0035, decision 6), and its optional features, each
// with the services, commands and channels that only it has, which the
// plugin uses only while the owner keeps the feature on (ADR-0055, decision
// 2). The Plugins page shows the same for each plugin, without the reasons
// and the arguments. `task plugins:grants` prints it.

import { compareText } from './manifests.mjs';
import { nativeGrants } from './native.mjs';

/**
 * A plugin's interface, as `task gen` writes it to
 * `.generated/desktop/interfaces.json`: every command and channel, and the
 * ones of each feature that its endpoints name.
 * @typedef {object} PluginInterface
 * @property {string[]} channels
 * @property {string[]} commands
 * @property {{ channels: string[], commands: string[], id: string }[]} features
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
 * Services with their majors, the sensitive ones marked.
 * @param {Record<string, number>} services
 * @param {string[]} sensitiveServices
 */
function serviceList(services, sensitiveServices) {
  return Object.entries(services).map(([name, major]) => {
    const mark = sensitiveServices.includes(name) ? ' (sensitive)' : '';
    return `${name} ${String(major)}${mark}`;
  });
}

/**
 * The lines of a plugin's optional features, in the order of their IDs:
 * each feature's ID, then the services that only it declares and the
 * commands and channels that only it handles.
 * @param {import('./manifests.mjs').Manifest} manifest
 * @param {PluginInterface} pluginInterface
 * @param {string[]} sensitiveServices
 * @returns {string[]}
 */
function featureLines(manifest, pluginInterface, sensitiveServices) {
  const features = Object.entries(manifest.features ?? {}).sort(([a], [b]) => compareText(a, b));
  if (features.length === 0) {
    return ['  Features: none'];
  }
  const lines = features.flatMap(([id, section]) => {
    const endpoints = pluginInterface.features.find((feature) => feature.id === id);
    return [
      `    ${id}`,
      `      Services: ${listOf(serviceList(section.services ?? {}, sensitiveServices))}`,
      `      Commands: ${listOf(endpoints?.commands ?? [])}`,
      `      Channels: ${listOf(endpoints?.channels ?? [])}`,
    ];
  });
  return ['  Features:', ...lines];
}

/**
 * What a plugin may use, as lines of text: a line with its ID and folder,
 * then one line for each kind of grant, and last its optional features. The
 * lines of the commands and channels name only the plugin's own, and each
 * feature's follow under it.
 * @param {import('./manifests.mjs').PluginFolder} plugin
 * @param {PluginInterface} pluginInterface
 * @param {string[]} sensitiveServices the services that reach sensitive data
 * @returns {string[]}
 */
export function grantLines(plugin, pluginInterface, sensitiveServices) {
  const { manifest } = plugin;
  const services = serviceList(manifest.services ?? {}, sensitiveServices);
  const ofFeatures = pluginInterface.features.flatMap((feature) => [...feature.commands, ...feature.channels]);
  const commands = pluginInterface.commands.filter((name) => !ofFeatures.includes(name));
  const channels = pluginInterface.channels.filter((name) => !ofFeatures.includes(name));
  const secrets = [
    ...(manifest.secrets?.entries ?? []).map((entry) => `${plugin.id}/${entry}`),
    ...(manifest.secrets?.legacy ?? []).map((name) => `${name} (a legacy name)`),
    ...(manifest.secrets?.keyed ?? []).map((family) => `${plugin.id}/${family}/<key> (a keyed family)`),
  ];
  const programs = (manifest.programs ?? []).map((program) => {
    const crates = program.crates ?? [];
    return crates.length > 0 ? `${program.package}, with ${crates.join(', ')}` : program.package;
  });
  const installed = Object.entries(manifest.processes ?? {}).flatMap(([program, process]) => [
    `    ${program}: ${process.reason}`,
    `      Fixed arguments: ${listOf(process.fixed ?? [], ' ')}`,
    `      Inherited variables: ${listOf(process.inherit ?? [])}`,
  ]);
  const native = nativeGrants(manifest).map((grant) => `    ${grant.category}: ${grant.reason}`);
  const where = plugin.validation ? `${plugin.folder}, in validation builds only` : plugin.folder;
  return [
    `${plugin.id} (${where})`,
    `  Services: ${listOf(services)}`,
    `  Secret entries: ${listOf(secrets)}`,
    `  Commands: ${listOf(commands)}`,
    `  Channels: ${listOf(channels)}`,
    `  Programs: ${listOf(programs, '; ')}`,
    ...(installed.length > 0 ? ['  Installed programs:', ...installed] : ['  Installed programs: none']),
    `  Crates that only it may use: ${listOf(manifest.crates?.exclusive ?? [])}`,
    ...(native.length > 0 ? ['  Native calls:', ...native] : ['  Native calls: none']),
    ...featureLines(manifest, pluginInterface, sensitiveServices),
  ];
}
