// @ts-check
// The Cargo workspaces of the repository (ADR-0035, decision 4) and the
// `cargo metadata` runs of `task gen`, whose failures become messages that
// name the task that fixes them.

import { spawnSync } from 'node:child_process';
import { existsSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

import { PLUGIN_FOLDERS } from './manifests.mjs';

/**
 * A Cargo workspace with a committed lock, or the generated launcher.
 * @typedef {object} Workspace
 * @property {string} folder its folder, relative to the repository's root, with forward slashes, `.` for the root
 * @property {'fixture' | 'launcher' | 'plugin' | 'root'} kind
 * @property {string} [plugin] the plugin's ID, for a plugin or a fixture plugin
 */

/**
 * The parts of `cargo metadata --format-version 1` that the scripts read.
 * @typedef {object} Metadata
 * @property {MetadataPackage[]} packages
 * @property {{ nodes: { deps: { dep_kinds: { kind: string | null, target: string | null }[], pkg: string }[], id: string }[] } | null} resolve
 * @property {string[]} workspace_members
 * @property {string} workspace_root
 */

/**
 * A package of `cargo metadata`.
 * @typedef {object} MetadataPackage
 * @property {{ kind: string | null, name: string, optional: boolean, path?: string, target: string | null }[]} dependencies
 * @property {string} id
 * @property {string} manifest_path
 * @property {string} name
 * @property {string | null} source
 * @property {{ kind: string[], name: string }[]} targets
 * @property {string} version
 */

/** The launcher that `task gen` writes, relative to the root. */
export const LAUNCHER_FOLDER = '.generated/desktop';

/**
 * The path of a workspace's manifest, relative to the root.
 * @param {Workspace} workspace
 */
export function manifestOf(workspace) {
  return workspace.folder === '.' ? 'Cargo.toml' : `${workspace.folder}/Cargo.toml`;
}

/**
 * The path of a workspace's lock, relative to the root.
 * @param {Workspace} workspace
 */
export function lockOf(workspace) {
  return workspace.folder === '.' ? 'Cargo.lock' : `${workspace.folder}/Cargo.lock`;
}

/**
 * The workspaces with committed locks: the root, then each plugin's and each
 * fixture plugin's, by folder. A folder counts when it has a `plugin.toml` and
 * a `Cargo.toml`; `task gen` reports the folders that miss one of them.
 * @param {string} root the repository's root
 * @returns {Workspace[]}
 */
export function committedWorkspaces(root) {
  /** @type {Workspace[]} */
  const workspaces = [{ folder: '.', kind: 'root' }];
  for (const { folder: parent, validation } of PLUGIN_FOLDERS) {
    if (!existsSync(join(root, parent))) {
      continue;
    }
    const names = readdirSync(join(root, parent), { withFileTypes: true })
      .filter((entry) => entry.isDirectory())
      .map((entry) => entry.name)
      .sort();
    for (const name of names) {
      const hasManifest = existsSync(join(root, parent, name, 'plugin.toml'));
      const hasWorkspace = existsSync(join(root, parent, name, 'Cargo.toml'));
      if (hasManifest && hasWorkspace) {
        workspaces.push({ folder: `${parent}/${name}`, kind: validation ? 'fixture' : 'plugin', plugin: name });
      }
    }
  }
  return workspaces;
}

/**
 * Runs `cargo metadata` on a workspace.
 * @param {string} root the repository's root, where cargo runs
 * @param {string} manifest the workspace's manifest, relative to the root
 * @param {{ locked: boolean, noDeps?: boolean, offline: boolean }} options `noDeps` reads the
 *   workspace's own packages without resolving its dependencies
 * @returns {{ metadata: Metadata, ok: true } | { ok: false, stderr: string }}
 */
export function cargoMetadata(root, manifest, options) {
  const args = ['metadata', '--format-version', '1', '--manifest-path', manifest];
  if (options.locked) {
    args.push('--locked');
  }
  if (options.noDeps === true) {
    args.push('--no-deps');
  }
  if (options.offline) {
    args.push('--offline');
  }
  const result = spawnSync('cargo', args, { cwd: root, encoding: 'utf8', maxBuffer: 1 << 30 });
  if (result.error !== undefined) {
    return { ok: false, stderr: `could not run cargo: ${result.error.message}` };
  }
  if (result.status !== 0) {
    return { ok: false, stderr: result.stderr.trim() };
  }
  return { metadata: JSON.parse(result.stdout), ok: true };
}

/**
 * The task that records a workspace's changed `Cargo.toml` in its lock.
 * @param {string | undefined} plugin the plugin's ID, or none for the root
 */
export function lockTask(plugin) {
  return plugin === undefined ? 'cargo update --workspace --offline' : `task plugins:lock ID=${plugin}`;
}

/**
 * The plugin whose crate needs a dependency, from the first
 * `required by package` line of cargo's error, or none for the core's.
 * @param {string} stderr
 * @returns {string | undefined}
 */
function requiringPlugin(stderr) {
  const line = /^required by package `([^`]+)`/mu.exec(stderr);
  if (line === null) {
    return undefined;
  }
  return /[\\/]plugins[\\/]([a-z0-9-]+)[\\/]/u.exec(line[1])?.[1];
}

/**
 * The versions at which the committed locks pin a crate, for messages.
 * @param {import('./lock.mjs').NamedLock[]} committed
 * @param {string} name
 */
function pinsOf(committed, name) {
  return committed.flatMap(({ lock, name: lockName }) =>
    lock.packages
      .filter((entry) => {
        const isCrate = entry.name === name;
        const isPinned = entry.source !== '';
        return isCrate && isPinned;
      })
      .map((entry) => `${entry.version} in ${lockName}`),
  );
}

/**
 * What a failed offline `cargo metadata` of a workspace means for the owner,
 * and the step that fixes it (ADR-0035, decision 5), for the forms of
 * Cargo's errors that spike S8 found: a lock that no longer matches its
 * manifest, or a requirement that no pinned version meets, needs the lock
 * task of the workspace; a crate that a committed lock pins but Cargo's cache
 * lacks needs `task setup`, which fetches every workspace; and a crate that
 * no committed lock pins needs the lock task of the plugin that asks for it,
 * which uses the network. Any other error is shown as cargo wrote it.
 * @param {Workspace} workspace
 * @param {string} stderr what cargo reported
 * @param {import('./lock.mjs').NamedLock[]} committed the committed locks
 */
export function explainMetadataFailure(workspace, stderr, committed) {
  const manifest = manifestOf(workspace);
  /**
   * A message with cargo's own error below it.
   * @param {string} message
   */
  const withCargo = (message) => `${message}\n${stderr}`;

  const isStaleLock = /cannot update the lock file|lock file needs to be updated/u.test(stderr);
  if (isStaleLock) {
    return withCargo(
      `${lockOf(workspace)} does not record every crate and dependency that ${manifest} names. Run \`${lockTask(workspace.plugin)}\`, then \`task gen\`.`,
    );
  }
  const download = /^error: failed to download `(\S+) v(\S+)`/mu.exec(stderr);
  const isDownload = download !== null;
  const isOffline = /--offline was specified/u.test(stderr);
  if (isDownload && isOffline) {
    return withCargo(
      `Cargo's cache lacks ${download[1]} ${download[2]}, which a committed lock pins. Run \`task setup\`, which downloads it, then \`task gen\`.`,
    );
  }
  const plugin = requiringPlugin(stderr) ?? workspace.plugin;
  const owner = plugin === undefined ? 'the core' : `the plugin ${plugin}`;
  const missing = /^error: no matching package named `(\S+)` found/mu.exec(stderr);
  if (missing !== null) {
    const pins = pinsOf(committed, missing[1]);
    if (pins.length > 0) {
      return withCargo(
        `Cargo's cache does not know ${missing[1]}, which the committed locks pin (${pins.join(', ')}). Run \`task setup\`, which downloads it, then \`task gen\`.`,
      );
    }
    return withCargo(
      `A crate of ${owner} depends on ${missing[1]}, which no committed lock pins and Cargo's cache does not hold. Run \`${lockTask(plugin)}\` with the network, then \`task gen\`.`,
    );
  }
  const select = /^error: failed to select a version for the requirement `(\S+) = "([^"]+)"`/mu.exec(stderr);
  if (select !== null) {
    const pins = pinsOf(committed, select[1]);
    const pinned = pins.length > 0 ? pins.join(', ') : 'no version of it';
    return withCargo(
      `A crate of ${owner} asks for ${select[1]} ${select[2]}, and the committed locks pin ${pinned}. Run \`${lockTask(plugin)}\`, then \`task gen\`.`,
    );
  }
  return withCargo(`cargo metadata failed for ${manifest}:`);
}
