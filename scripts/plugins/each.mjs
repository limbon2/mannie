// @ts-check
// Runs one command in every Cargo workspace that has a committed lock: the
// root, then each plugin's and each fixture plugin's (ADR-0035, decision 4),
// so the Taskfile names no plugin. `{manifest}` in the command becomes the
// workspace's `Cargo.toml` and `{folder}` its folder, both relative to the
// root, where the command runs. Every workspace runs even after one fails, so
// one run reports every failure, and the exit code is 1 when any failed.
//
// Usage:
//   node scripts/plugins/each.mjs [--launcher | --plugins] -- <command> [arguments]
//   node scripts/plugins/each.mjs [--launcher | --plugins] --list
//
// `--launcher` adds the launcher, `.generated/desktop/`, last, which needs
// `task gen` first. `--plugins` leaves the root out. `--list` prints the
// manifests, one on each line.

import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';

import { committedWorkspaces, LAUNCHER_FOLDER, manifestOf } from './lib/cargo.mjs';

/** @typedef {import('./lib/cargo.mjs').Workspace} Workspace */

const root = resolve(import.meta.dirname, '../..');
const args = process.argv.slice(2);
const separator = args.indexOf('--');
const options = separator === -1 ? args : args.slice(0, separator);
const command = separator === -1 ? [] : args.slice(separator + 1);

const known = new Set(['--launcher', '--list', '--plugins']);
const unknown = options.filter((option) => !known.has(option));
const hasUnknown = unknown.length > 0;
const lists = options.includes('--list');
// Exactly one of a command and --list.
const hasCommand = command.length > 0;
const isAmbiguous = hasCommand === lists;
if (hasUnknown || isAmbiguous) {
  console.error('usage: node scripts/plugins/each.mjs [--launcher | --plugins] (--list | -- <command> [arguments])');
  process.exit(2);
}

const onlyPlugins = options.includes('--plugins');
/** @type {Workspace[]} */
const workspaces = committedWorkspaces(root).filter((workspace) => {
  const isRoot = workspace.kind === 'root';
  return !(onlyPlugins && isRoot);
});
if (options.includes('--launcher')) {
  workspaces.push({ folder: LAUNCHER_FOLDER, kind: 'launcher' });
}

if (lists) {
  for (const workspace of workspaces) {
    console.log(manifestOf(workspace));
  }
  process.exit(0);
}

/** @type {string[]} */
const failed = [];
for (const workspace of workspaces) {
  const filled = command.map((part) =>
    part.replaceAll('{manifest}', manifestOf(workspace)).replaceAll('{folder}', workspace.folder),
  );
  console.log(`each: ${filled.join(' ')}`);
  const result = spawnSync(filled[0], filled.slice(1), { cwd: root, stdio: 'inherit' });
  if (result.error !== undefined) {
    console.error(`each: could not run ${filled[0]}: ${result.error.message}`);
    failed.push(workspace.folder);
  } else if (result.status !== 0) {
    failed.push(workspace.folder);
  }
}
if (failed.length > 0) {
  console.error(`each: the command failed in ${failed.join(', ')}`);
  process.exitCode = 1;
}
