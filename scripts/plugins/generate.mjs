// @ts-check
// The generator of `task gen` (ADR-0035, decisions 3 to 6 and 12).
//
// Without an option, the first step of `task gen`:
//
// 1. finds the plugins, `plugins/<id>/plugin.toml`, and the fixture plugins of
//    `tests/fixtures/plugins/`, and checks their manifests and workspaces;
// 2. checks every committed lock against its workspace with
//    `cargo metadata --locked --offline`, and that the committed locks agree;
// 3. writes the launcher, `.generated/desktop/`: its workspace, the plugin
//    list crate and the plugin bindings program;
// 4. seeds the launcher's lock from the committed locks, normalizes it with
//    `cargo metadata --offline` and checks that each of its packages equals a
//    package of a committed lock;
// 5. writes the index of the plugins and the editor workspace.
//
// It writes a file only when its content changed, so `tauri dev` does not
// build again for nothing, and it writes nothing when a check fails.
//
// `--seal`, the last step of `task gen`, checks the boundaries between the
// core and the plugins and then writes the fingerprint that the desktop
// binary's build script checks. `--check`, which `task lint:plugins` runs,
// writes nothing and reports every generated file that `task gen` would
// change.
//
// Usage: node scripts/plugins/generate.mjs [--seal | --check]

import { existsSync, readFileSync, rmSync, statSync, utimesSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

import { checkBoundaries } from './lib/boundaries.mjs';
import { cargoMetadata, explainMetadataFailure, manifestOf } from './lib/cargo.mjs';
import { fingerprint } from './lib/fingerprint.mjs';
import {
  FINGERPRINT_FILE,
  generatedFiles,
  isCurrent,
  LAUNCHER_LOCK,
  LAUNCHER_WORKSPACE,
  readInputs,
  writeIfChanged,
} from './lib/generator.mjs';
import { pinProblems, readLock, renderLock, seedLock } from './lib/lock.mjs';

/**
 * @typedef {import('./lib/generator.mjs').Inputs} Inputs
 */

const root = resolve(import.meta.dirname, '../..');

/**
 * Stops with the problems that a step found.
 * @param {string[]} problems
 * @param {string} outcome what the failure left, such as "nothing was written"
 * @returns {never}
 */
function fail(problems, outcome) {
  for (const problem of problems) {
    console.error(`error: ${problem}`);
  }
  console.error(`generate: ${String(problems.length)} problem(s); ${outcome}.`);
  process.exit(1);
}

/**
 * Why the packages of a launcher's lock are not all pinned by the committed
 * locks, with the tasks that pin them.
 * @param {Inputs} inputs
 */
function launcherLockProblems(inputs) {
  return pinProblems(readLock(join(root, LAUNCHER_LOCK)), inputs.committedLocks).map(
    (problem) =>
      `${problem}. Run \`task plugins:lock ID=<id>\` for the plugin that needs it, or \`task deps:update\` for a crate of the core.`,
  );
}

/**
 * Seeds the launcher's lock, normalizes it offline and checks its pins. The
 * previous lock comes back when a step fails, and keeps its time when the
 * result equals it.
 * @param {Inputs} inputs
 * @returns {boolean} whether the lock changed
 */
function writeLauncherLock(inputs) {
  const path = join(root, LAUNCHER_LOCK);
  const previous = existsSync(path) ? { stat: statSync(path), text: readFileSync(path, 'utf8') } : null;
  const restorePrevious = () => {
    if (previous === null) {
      rmSync(path, { force: true });
    } else {
      writeFileSync(path, previous.text);
    }
  };
  writeFileSync(path, renderLock(seedLock(inputs.committedLocks.map((named) => named.lock))));
  const normalized = cargoMetadata(root, manifestOf(LAUNCHER_WORKSPACE), { locked: false, offline: true });
  if (!normalized.ok) {
    restorePrevious();
    fail([explainMetadataFailure(LAUNCHER_WORKSPACE, normalized.stderr, inputs.committedLocks)], 'the lock was kept');
  }
  const unpinned = launcherLockProblems(inputs);
  if (unpinned.length > 0) {
    restorePrevious();
    fail(unpinned, 'the lock was kept');
  }
  const text = readFileSync(path, 'utf8');
  const hasPrevious = previous !== null;
  const isUnchanged = previous?.text === text;
  if (hasPrevious && isUnchanged) {
    utimesSync(path, previous.stat.atime, previous.stat.mtime);
    return false;
  }
  return true;
}

/** Writes every generated file of the first step of `task gen`. */
function generate() {
  const read = readInputs(root);
  if (read.inputs === null) {
    fail(read.problems, 'nothing was written');
  }
  const { inputs } = read;
  const files = generatedFiles(root, inputs);
  // The launcher's workspace goes first, because the lock's normalization reads it.
  const written = files.filter((file) => writeIfChanged(root, file)).map((file) => file.path);
  if (writeLauncherLock(inputs)) {
    written.push(LAUNCHER_LOCK);
  }
  const ids = inputs.launcherPlugins.map((plugin) => (plugin.validation ? `${plugin.id} (validation)` : plugin.id));
  const pinned = readLock(join(root, LAUNCHER_LOCK)).packages.filter((entry) => entry.source !== '').length;
  const changes = written.length === 0 ? 'nothing changed' : `wrote ${written.join(', ')}`;
  console.log(
    `generate: ${String(ids.length)} plugins (${ids.join(', ')}); the launcher's lock pins ${String(pinned)} crates from ${String(inputs.committedLocks.length)} committed locks; ${changes}`,
  );
}

/** Checks the boundaries, then writes the fingerprint. */
function seal() {
  const { graphs, problems } = checkBoundaries(root);
  if (problems.length > 0) {
    fail(problems, 'the fingerprint was not written, so the desktop binary does not build');
  }
  const wrote = writeIfChanged(root, { path: FINGERPRINT_FILE, text: fingerprint(root) });
  console.log(
    `generate: the boundaries of ${String(graphs)} graphs hold; ${wrote ? `wrote ${FINGERPRINT_FILE}` : 'the fingerprint did not change'}`,
  );
}

/** Reports every generated file that `task gen` would change. */
function check() {
  const read = readInputs(root);
  if (read.inputs === null) {
    fail(read.problems, 'the generated files were not checked');
  }
  const { inputs } = read;
  const stale = generatedFiles(root, inputs)
    .filter((file) => !isCurrent(root, file))
    .map((file) => `${file.path} is out of date. Run \`task gen\`.`);
  if (existsSync(join(root, LAUNCHER_LOCK))) {
    const locked = cargoMetadata(root, manifestOf(LAUNCHER_WORKSPACE), { locked: true, offline: true });
    if (locked.ok) {
      stale.push(...launcherLockProblems(inputs));
    } else {
      stale.push(`${LAUNCHER_LOCK} does not match the launcher. Run \`task gen\`.`);
    }
  } else {
    stale.push(`${LAUNCHER_LOCK} is missing. Run \`task gen\`.`);
  }
  if (!isCurrent(root, { path: FINGERPRINT_FILE, text: fingerprint(root) })) {
    stale.push(`${FINGERPRINT_FILE} is out of date: the plugins changed since \`task gen\` ran. Run \`task gen\`.`);
  }
  if (stale.length > 0) {
    fail(stale, 'nothing was written');
  }
  console.log('generate --check: the generated files are up to date');
}

const mode = process.argv[2];
switch (mode) {
  case undefined: {
    generate();
    break;
  }
  case '--seal': {
    seal();
    break;
  }
  case '--check': {
    check();
    break;
  }
  default: {
    console.error('usage: node scripts/plugins/generate.mjs [--seal | --check]');
    process.exit(2);
  }
}
