// @ts-check
// The fingerprint of what `task gen` read (ADR-0035, decision 3): the SHA-256
// of every file whose change needs a new launcher, a new launcher lock or
// new grants, one line each, `<sha256>  <path>`, sorted by path. `task gen`
// writes it last, after the boundary check, and the desktop binary's build
// script computes the same lines and refuses to build when they differ, with
// "The plugins changed since `task gen` ran. Run `task gen`."
//
// The files: the root's `Cargo.toml` and `Cargo.lock`, the manifests of the
// desktop app's two crates, the plugins' interfaces in the launcher, and in
// each folder of `plugins/` and `tests/fixtures/plugins/` that has a
// `plugin.toml`, that file, its `Cargo.lock` and every `Cargo.toml` below it,
// outside folders named `target`, `node_modules` or `generated` and folders
// whose names start with a dot. The build script lists them the same way.

import { createHash } from 'node:crypto';
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

import { PLUGIN_FOLDERS } from './manifests.mjs';

/** The files outside the plugins' folders that the fingerprint covers. */
export const FINGERPRINTED_FILES = [
  'Cargo.toml',
  'Cargo.lock',
  'apps/desktop/core/Cargo.toml',
  'apps/desktop/src-tauri/Cargo.toml',
  '.generated/desktop/interfaces.json',
];

/** Folders that the walk of a plugin's folder skips. */
const SKIPPED_FOLDERS = new Set(['generated', 'node_modules', 'target']);

/**
 * The `Cargo.toml` files below a folder, relative to the root.
 * @param {string} root
 * @param {string} folder relative to the root
 * @returns {string[]}
 */
function cargoManifests(root, folder) {
  /** @type {string[]} */
  const found = [];
  for (const entry of readdirSync(join(root, folder), { withFileTypes: true })) {
    const path = `${folder}/${entry.name}`;
    const isSkipped = SKIPPED_FOLDERS.has(entry.name) || entry.name.startsWith('.');
    // As the build script decides: a folder is walked unless skipped, and
    // anything else named Cargo.toml counts, a link to a file included.
    if (entry.isDirectory()) {
      if (!isSkipped) {
        found.push(...cargoManifests(root, path));
      }
    } else if (entry.name === 'Cargo.toml') {
      found.push(path);
    }
  }
  return found;
}

/**
 * The files that the fingerprint covers, relative to the root, sorted.
 * @param {string} root the repository's root
 * @returns {string[]}
 */
export function fingerprintedFiles(root) {
  const files = [...FINGERPRINTED_FILES];
  for (const { folder: parent } of PLUGIN_FOLDERS) {
    if (!existsSync(join(root, parent))) {
      continue;
    }
    for (const entry of readdirSync(join(root, parent), { withFileTypes: true })) {
      const folder = `${parent}/${entry.name}`;
      const isPlugin = entry.isDirectory() && existsSync(join(root, folder, 'plugin.toml'));
      if (!isPlugin) {
        continue;
      }
      files.push(`${folder}/plugin.toml`, `${folder}/Cargo.lock`, ...cargoManifests(root, folder));
    }
  }
  return [...new Set(files)].sort();
}

/**
 * The fingerprint's text: a line for each file with its SHA-256, or `missing`
 * for a file that does not exist.
 * @param {string} root the repository's root
 */
export function fingerprint(root) {
  const lines = fingerprintedFiles(root).map((file) => {
    const path = join(root, file);
    const hash = existsSync(path) ? createHash('sha256').update(readFileSync(path)).digest('hex') : 'missing';
    return `${hash}  ${file}`;
  });
  return `${lines.join('\n')}\n`;
}
