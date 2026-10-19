// @ts-check
// Prints what each plugin may use, for the review of a plugin's change
// (ADR-0034, decision 13; scripts/plugins/lib/grants.mjs): its services, its
// secret entries, its commands and channels, its programs, the crates that
// only it may use and its direct native calls. The commands and channels
// come from `.generated/desktop/interfaces.json`, so `task plugins:grants`
// runs `task gen` first.
//
// Usage: node scripts/plugins/grants.mjs

import { existsSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

import { LAUNCHER_FOLDER } from './lib/cargo.mjs';
import { grantLines } from './lib/grants.mjs';
import { findPlugins, readRules } from './lib/manifests.mjs';

/** @typedef {import('./lib/grants.mjs').PluginInterface} PluginInterface */

const root = resolve(import.meta.dirname, '../..');
const rules = readRules(root);
const found = findPlugins(root, rules);
for (const problem of found.problems) {
  console.error(`error: ${problem}`);
}
if (found.problems.length > 0) {
  process.exit(1);
}

const interfacesFile = join(root, LAUNCHER_FOLDER, 'interfaces.json');
if (!existsSync(interfacesFile)) {
  console.error(`error: ${LAUNCHER_FOLDER}/interfaces.json is missing. Run \`task gen\`.`);
  process.exit(1);
}
/** @type {PluginInterface[]} */
const interfaces = JSON.parse(readFileSync(interfacesFile, 'utf8'));

/** @type {string[]} */
const blocks = [];
for (const plugin of found.plugins) {
  const pluginInterface = interfaces.find((candidate) => candidate.id === plugin.id);
  if (pluginInterface === undefined) {
    console.error(`error: ${LAUNCHER_FOLDER}/interfaces.json has no interface of ${plugin.id}. Run \`task gen\`.`);
    process.exit(1);
  }
  blocks.push(grantLines(plugin, pluginInterface, rules.sensitiveServices).join('\n'));
}
console.log(blocks.join('\n\n'));
