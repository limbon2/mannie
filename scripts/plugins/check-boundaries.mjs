// @ts-check
// Checks the boundaries between the core and the plugins (ADR-0034, decision
// 2; ADR-0035, decision 6) in the graphs of every Cargo workspace and of the
// launcher, as scripts/plugins/lib/boundaries.mjs describes them. `task gen`
// runs the same check before it writes the fingerprint, and `task lint:deps`
// runs this script.
//
// Usage: node scripts/plugins/check-boundaries.mjs

import { resolve } from 'node:path';

import { checkBoundaries } from './lib/boundaries.mjs';

const root = resolve(import.meta.dirname, '../..');
const { graphs, problems } = checkBoundaries(root);
for (const problem of problems) {
  console.error(`error: ${problem}`);
}
if (problems.length > 0) {
  process.exitCode = 1;
} else {
  console.log(`check-boundaries: the dependencies of ${String(graphs)} graphs keep the core and the plugins apart`);
}
