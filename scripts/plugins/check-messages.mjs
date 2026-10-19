// @ts-check
// Checks the message catalogs of the core and of every plugin, the fixture
// plugins' included, for what inlang does not report (ADR-0035, decision 8):
// a plugin's key outside its prefix, a core key under a plugin's prefix, a
// key in two sources, a plugin without its name or description, and a
// plugin's catalog for a locale that the core does not register. `task gen`
// and `task lint:messages` run it.
//
// Usage: node scripts/plugins/check-messages.mjs

import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

import { findPlugins, readRules } from './lib/manifests.mjs';
import { APP_FOLDER, messageProblems, messageSources, readCatalogs } from './lib/messages.mjs';

const root = resolve(import.meta.dirname, '../..');
const found = findPlugins(root, readRules(root));
const settings = JSON.parse(readFileSync(join(root, APP_FOLDER, 'project.inlang', 'settings.json'), 'utf8'));
const sources = messageSources(found.plugins);
const catalogs = new Map(sources.map((source) => [source.name, readCatalogs(root, source)]));
const problems = [...found.problems, ...messageProblems(sources, catalogs, settings)];
for (const problem of problems) {
  console.error(`error: ${problem}`);
}
if (problems.length > 0) {
  process.exitCode = 1;
} else {
  const keys = [...catalogs.values()].reduce((count, byLocale) => {
    const base = byLocale.get(settings.baseLocale) ?? {};
    return count + Object.keys(base).filter((key) => key !== '$schema').length;
  }, 0);
  console.log(`check-messages: ${String(keys)} messages in ${String(sources.length)} sources keep to their prefixes`);
}
