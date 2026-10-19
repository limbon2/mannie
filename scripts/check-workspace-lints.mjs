// @ts-check
// Checks the lint policy of every Cargo workspace (see Cargo.toml): every
// crate of the root's workspace, of each plugin's, of each fixture plugin's
// and of the launcher inherits its workspace's lints with
// `[lints] workspace = true`, and every one of those workspaces repeats the
// root's `[workspace.package]` and `[workspace.lints]` (ADR-0035, decision
// 4). A crate without the setting compiles under Rust's default lints, and a
// workspace with other lints, and Clippy would pass either silently.
// `task lint:rust` runs it; the launcher is checked once `task gen` wrote it.

import { existsSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

import { parse } from 'smol-toml';

import { cargoMetadata, committedWorkspaces, LAUNCHER_FOLDER, manifestOf } from './plugins/lib/cargo.mjs';

/** @typedef {import('./plugins/lib/cargo.mjs').Workspace} Workspace */

const root = resolve(import.meta.dirname, '..');

// The three ways TOML can spell the setting.
const inheritsWorkspaceLints = [
  /^\[lints\][ \t]*\r?\n(?:[ \t]*(?:#.*)?\r?\n)*[ \t]*workspace[ \t]*=[ \t]*true\b/m,
  /^lints\.workspace[ \t]*=[ \t]*true\b/m,
  /^lints[ \t]*=[ \t]*\{[ \t]*workspace[ \t]*=[ \t]*true[ \t]*\}/m,
];

/**
 * A value as JSON with the keys of every table sorted, so two tables compare
 * equal whatever order their keys have.
 * @param {unknown} value
 * @returns {string}
 */
function canonical(value) {
  if (Array.isArray(value)) {
    return `[${value.map(canonical).join(',')}]`;
  }
  const isTable = typeof value === 'object' && value !== null;
  if (isTable) {
    const entries = Object.entries(value).sort(([a], [b]) => a.localeCompare(b));
    return `{${entries.map(([key, field]) => `${JSON.stringify(key)}:${canonical(field)}`).join(',')}}`;
  }
  return JSON.stringify(value);
}

/**
 * The `[workspace]` table of a manifest.
 * @param {string} manifest relative to the root
 * @returns {Record<string, unknown>}
 */
function workspaceTable(manifest) {
  const parsed = parse(readFileSync(join(root, manifest), 'utf8'));
  return /** @type {Record<string, unknown>} */ (parsed.workspace ?? {});
}

/** @type {Workspace[]} */
const workspaces = committedWorkspaces(root);
if (existsSync(join(root, LAUNCHER_FOLDER, 'Cargo.toml'))) {
  workspaces.push({ folder: LAUNCHER_FOLDER, kind: 'launcher' });
}

const rootWorkspace = workspaceTable('Cargo.toml');
/** @type {string[]} */
const problems = [];
let crates = 0;
for (const workspace of workspaces) {
  const manifest = manifestOf(workspace);
  if (workspace.kind !== 'root') {
    const table = workspaceTable(manifest);
    for (const key of ['package', 'lints']) {
      const isEqual = canonical(table[key]) === canonical(rootWorkspace[key]);
      if (!isEqual) {
        problems.push(`${manifest}: [workspace.${key}] must equal the root's, as in Cargo.toml`);
      }
    }
  }
  const result = cargoMetadata(root, manifest, { locked: false, noDeps: true, offline: true });
  if (!result.ok) {
    problems.push(`${manifest}: cargo metadata failed:\n${result.stderr}`);
    continue;
  }
  for (const crate of result.metadata.packages) {
    crates += 1;
    const text = readFileSync(crate.manifest_path, 'utf8');
    if (!inheritsWorkspaceLints.some((pattern) => pattern.test(text))) {
      problems.push(
        `${crate.manifest_path}: add "[lints] workspace = true" so ${crate.name} follows the workspace lints`,
      );
    }
  }
}

for (const problem of problems) {
  console.error(`error: ${problem}`);
}
if (problems.length > 0) {
  process.exitCode = 1;
} else {
  console.log(
    `check-workspace-lints: ${String(crates)} crates in ${String(workspaces.length)} workspaces inherit the workspace lints, and the workspaces repeat the root's settings`,
  );
}
