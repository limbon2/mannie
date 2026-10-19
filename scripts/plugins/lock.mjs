// @ts-check
// `task plugins:lock ID=<id>` (ADR-0035, decision 5): records a plugin's
// changed `Cargo.toml` in its `Cargo.lock` with `cargo update --workspace`,
// which adds new members and crates and keeps every other pin. Cargo's cache
// is tried first, and the network only when it lacks a crate. A plugin
// without a lock gets one seeded from the root's, so the crates that both use
// keep the root's versions. It works the same for a fixture plugin in
// `tests/fixtures/plugins/`.
//
// Usage: node scripts/plugins/lock.mjs <id>

import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync } from 'node:fs';
import { join, resolve } from 'node:path';

import { PLUGIN_FOLDERS, readRules } from './lib/manifests.mjs';

const root = resolve(import.meta.dirname, '../..');
const id = process.argv[2] ?? '';
const rules = readRules(root);
if (!rules.idPattern.test(id)) {
  console.error("usage: node scripts/plugins/lock.mjs <id>, where <id> is the name of a plugin's folder");
  process.exit(2);
}
const folder = PLUGIN_FOLDERS.map((parent) => `${parent.folder}/${id}`).find((candidate) =>
  existsSync(join(root, candidate, 'Cargo.toml')),
);
if (folder === undefined) {
  console.error(
    `error: no plugin ${id} with a Cargo.toml in ${PLUGIN_FOLDERS.map((parent) => parent.folder).join(' or ')}`,
  );
  process.exit(1);
}

const lock = join(root, folder, 'Cargo.lock');
if (!existsSync(lock)) {
  copyFileSync(join(root, 'Cargo.lock'), lock);
  console.log(`lock: seeded ${folder}/Cargo.lock from the root's lock`);
}
const args = ['update', '--workspace', '--manifest-path', `${folder}/Cargo.toml`];
const offline = spawnSync('cargo', [...args, '--offline'], { cwd: root, encoding: 'utf8' });
if (offline.status === 0) {
  process.stderr.write(offline.stderr);
  console.log(`lock: recorded ${folder}/Cargo.toml in its lock`);
  process.exit(0);
}
const lacksCrates = /offline/u.test(offline.stderr);
if (!lacksCrates) {
  process.stderr.write(offline.stderr);
  process.exit(1);
}
console.log("lock: Cargo's cache lacks a crate, so cargo runs again with the network");
const online = spawnSync('cargo', args, { cwd: root, stdio: 'inherit' });
if (online.status !== 0) {
  process.exit(1);
}
console.log(`lock: recorded ${folder}/Cargo.toml in its lock`);
