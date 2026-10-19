// @ts-check
// `task plugins:new ID=<id>` (ADR-0035, decision 13): a new plugin in
// `plugins/<id>/`, made from the fixture plugin as a template, with a lock
// seeded from the root's. `task gen` then bundles it, and nothing outside
// its folder changes.
//
// Usage: node scripts/plugins/new.mjs <id>

import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';

import { checkId, findPlugins, prefixesNest, prefixOf, readRules } from './lib/manifests.mjs';
import { renamedPath, TEMPLATE_FOLDER, templateFile } from './lib/template.mjs';

const root = resolve(import.meta.dirname, '../..');
const id = process.argv[2] ?? '';
const rules = readRules(root);

/**
 * Stops with a problem.
 * @param {string} problem
 * @returns {never}
 */
function fail(problem) {
  console.error(`error: ${problem}`);
  process.exit(1);
}

const idProblem = checkId(id, rules);
if (idProblem !== null) {
  fail(idProblem.message);
}
const folder = `plugins/${id}`;
if (existsSync(join(root, folder))) {
  fail(`${folder} exists already`);
}
const nesting = findPlugins(root, rules).plugins.find((plugin) => prefixesNest(prefixOf(plugin.id), prefixOf(id)));
if (nesting !== undefined) {
  fail(
    `the prefix of ${id} nests with the prefix of the plugin ${nesting.id}, so their tables could not be told apart`,
  );
}

/** Folders and files of the template that the new plugin does not take. */
const SKIPPED = new Set(['Cargo.lock', 'generated', 'node_modules', 'target']);

/**
 * The template's files, relative to its folder.
 * @param {string} relativeFolder
 * @returns {string[]}
 */
function templateFiles(relativeFolder) {
  return readdirSync(join(root, TEMPLATE_FOLDER, relativeFolder), { withFileTypes: true }).flatMap((entry) => {
    const path = relativeFolder === '' ? entry.name : `${relativeFolder}/${entry.name}`;
    if (SKIPPED.has(entry.name)) {
      return [];
    }
    return entry.isDirectory() ? templateFiles(path) : [path];
  });
}

for (const path of templateFiles('')) {
  const text = readFileSync(join(root, TEMPLATE_FOLDER, path), 'utf8');
  const target = join(root, folder, renamedPath(path, id));
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, templateFile(path, text, id));
}
console.log(`new: wrote ${folder}/ from ${TEMPLATE_FOLDER}/`);

// The new names change the lengths of lines and the order of sorted names,
// so the formatters lay the files out again. The ID is checked above, so it
// is safe in a command line.
/** @type {[string, string[]][]} */
const formatters = [
  ['cargo', ['fmt', '--all', '--manifest-path', `${folder}/Cargo.toml`]],
  ['pnpm', ['exec', 'tombi', 'format', '--quiet', folder]],
  ['pnpm', ['exec', 'prettier', '--write', '--log-level', 'warn', folder]],
];
for (const [command, args] of formatters) {
  // pnpm is a script on Windows, which runs only through a shell. The shell
  // gets one line, since Node warns about arguments that a shell joins
  // unescaped (DEP0190).
  const line = [command, ...args].join(' ');
  const formatted = spawnSync(line, { cwd: root, shell: true, stdio: 'inherit' });
  if (formatted.status !== 0) {
    fail(`${line} failed`);
  }
}

// The lock: seeded from the root's, with the new crates recorded.
const lock = spawnSync('node', [join(root, 'scripts/plugins/lock.mjs'), id], { cwd: root, stdio: 'inherit' });
if (lock.status !== 0) {
  fail(`could not write ${folder}/Cargo.lock; run \`task plugins:lock ID=${id}\` once the problem above is fixed`);
}
console.log(`new: the plugin ${id} is ready. Run \`task gen\` to bundle it, and turn it on in the Plugins settings.`);
