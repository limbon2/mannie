// @ts-check
// `task deps:align` (ADR-0035, decision 5): brings every crate that the
// committed locks pin within one compatible range at different versions to
// one version, the root's where the root pins the crate and otherwise the
// newest, with `cargo update -p <name>@<old> --precise <new>` in each lock
// that differs. Only the plugins' and the fixture plugins' locks change,
// because the root's version always wins. Cargo's cache is tried first, and
// the network only when it lacks a crate.
//
// Usage: node scripts/plugins/align-locks.mjs

import { spawnSync } from 'node:child_process';
import { join, resolve } from 'node:path';

import { committedWorkspaces, lockOf, manifestOf } from './lib/cargo.mjs';
import { compareVersions, disagreements, readLock } from './lib/lock.mjs';

const root = resolve(import.meta.dirname, '../..');

/** The committed locks, the root's first, with each workspace. */
function committed() {
  return committedWorkspaces(root).map((workspace) => ({
    lock: readLock(join(root, lockOf(workspace))),
    name: lockOf(workspace),
    workspace,
  }));
}

/**
 * Runs cargo in the root, offline first and with the network when Cargo's
 * cache lacks a crate.
 * @param {string[]} args
 * @returns {boolean} whether cargo succeeded
 */
function cargoWithCache(args) {
  const offline = spawnSync('cargo', [...args, '--offline'], { cwd: root, encoding: 'utf8' });
  if (offline.status === 0) {
    return true;
  }
  const lacksCrates = /offline/u.test(offline.stderr);
  if (!lacksCrates) {
    process.stderr.write(offline.stderr);
    return false;
  }
  console.log("align: Cargo's cache lacks a crate, so cargo runs again with the network");
  const online = spawnSync('cargo', args, { cwd: root, stdio: 'inherit' });
  return online.status === 0;
}

const locks = committed();
const rootLock = locks[0].name;
const found = disagreements(locks);
if (found.length === 0) {
  console.log(`align: the ${String(locks.length)} committed locks agree`);
  process.exit(0);
}

let failures = 0;
for (const crate of found) {
  const rootPin = crate.pins.find((pin) => pin.lock === rootLock);
  const newest =
    crate.pins
      .map((pin) => pin.version)
      .sort(compareVersions)
      .at(-1) ?? '';
  const target = rootPin?.version ?? newest;
  for (const pin of crate.pins.filter((candidate) => candidate.version !== target)) {
    const workspace = locks.find((lock) => lock.name === pin.lock)?.workspace;
    if (workspace === undefined) {
      continue;
    }
    console.log(`align: ${crate.name} ${pin.version} -> ${target} in ${pin.lock}`);
    const args = [
      'update',
      '--manifest-path',
      manifestOf(workspace),
      '-p',
      `${crate.name}@${pin.version}`,
      '--precise',
      target,
    ];
    if (!cargoWithCache(args)) {
      console.error(`error: could not bring ${crate.name} to ${target} in ${pin.lock}`);
      failures += 1;
    }
  }
}

const remaining = disagreements(committed());
for (const crate of remaining) {
  console.error(
    `error: the locks still pin ${crate.name} ${crate.range} at ${crate.pins.map((pin) => `${pin.version} in ${pin.lock}`).join(', ')}`,
  );
}
const hasFailures = failures > 0;
const stillDisagree = remaining.length > 0;
if (hasFailures || stillDisagree) {
  process.exitCode = 1;
} else {
  console.log(`align: the ${String(locks.length)} committed locks agree`);
}
