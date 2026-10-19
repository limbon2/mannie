// @ts-check
// Runs Clippy on the library crates that each plugin's entry crate ships,
// with the core's rule against direct native calls: file writes, processes
// and sockets that do not go through the SDK's services (ADR-0035, decision
// 6; scripts/plugins/lib/native.mjs). Each plugin gets the root's
// clippy.toml with the calls of every category that its manifest does not
// grant under [native], which this script writes to
// .generated/clippy/<id>/clippy.toml and passes in CLIPPY_CONF_DIR. Clippy
// tracks that variable and the file, so a changed grant checks the crates
// again. The crates build with their default features, as the desktop app
// links them.
//
// Clippy only warns about a path of the configuration that does not
// resolve, which `-D warnings` leaves a warning, so this script fails on that
// warning: a mistyped path would refuse nothing. It also refuses a
// suppression of the rule in the crates' sources, so that a grant stays the
// only exception. Every plugin runs even after one fails, so one run reports
// every failure. `task lint:rust` runs this script.
//
// Usage: node scripts/plugins/check-native.mjs

import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';

import { boundaryPlugin, rustFiles, shippedLibraryPackages } from './lib/boundaries.mjs';
import { cargoMetadata, committedWorkspaces, explainMetadataFailure, lockOf, manifestOf } from './lib/cargo.mjs';
import { writeIfChanged } from './lib/generator.mjs';
import { readLock } from './lib/lock.mjs';
import { findPlugins, readRules } from './lib/manifests.mjs';
import { nativeGrants, nativeSuppressions, pluginClippyConfiguration } from './lib/native.mjs';

/** @typedef {import('./lib/cargo.mjs').MetadataPackage} MetadataPackage */

const root = resolve(import.meta.dirname, '../..');
const found = findPlugins(root, readRules(root));
for (const problem of found.problems) {
  console.error(`error: ${problem}`);
}
if (found.problems.length > 0) {
  process.exit(1);
}

/**
 * The suppressions of the rule in the sources of a crate's `src/`, as
 * messages that name the file and the line.
 * @param {MetadataPackage} entry
 * @param {string} id the plugin's ID
 * @returns {string[]}
 */
function suppressionProblems(entry, id) {
  const sources = join(dirname(entry.manifest_path), 'src');
  if (!existsSync(sources)) {
    return [];
  }
  return rustFiles(sources).flatMap((file) =>
    nativeSuppressions(readFileSync(file, 'utf8')).map(({ line, lint }) => {
      const where = `${relative(root, file).replaceAll('\\', '/')}:${String(line)}`;
      return `${where}: ${lint} suppresses the rule against direct native calls in a library crate that ${id} ships; grant the category under [native] in its plugin.toml instead`;
    }),
  );
}

/**
 * The paths of a Clippy configuration that Clippy reported as not
 * resolving, from its output.
 * @param {string} output
 * @returns {string[]}
 */
function unresolvedPaths(output) {
  const paths = [...output.matchAll(/^warning: `([^`]+)` does not refer to /gmu)].map((match) => match[1]);
  return [...new Set(paths)];
}

const rootConfiguration = readFileSync(join(root, 'clippy.toml'), 'utf8');
const workspaces = committedWorkspaces(root);
const committed = workspaces.map((workspace) => ({
  lock: readLock(join(root, lockOf(workspace))),
  name: lockOf(workspace),
}));
/** @type {string[]} */
const failed = [];
for (const plugin of found.plugins) {
  /** @type {import('./lib/cargo.mjs').Workspace} */
  const workspace = workspaces.find((candidate) => candidate.folder === plugin.folder) ?? {
    folder: plugin.folder,
    kind: plugin.validation ? 'fixture' : 'plugin',
    plugin: plugin.id,
  };
  const manifest = manifestOf(workspace);
  const metadata = cargoMetadata(root, manifest, { locked: true, offline: true });
  if (!metadata.ok) {
    console.error(`error: ${explainMetadataFailure(workspace, metadata.stderr, committed)}`);
    failed.push(plugin.id);
    continue;
  }
  const packages = shippedLibraryPackages(metadata.metadata, boundaryPlugin(plugin), root);
  if (packages.length === 0) {
    console.error(`error: ${manifest} has no package ${plugin.manifest.rust.entry}, the entry crate of ${plugin.id}`);
    failed.push(plugin.id);
    continue;
  }
  const names = packages.map((entry) => entry.name);
  const granted = nativeGrants(plugin.manifest).map((grant) => grant.category);
  const grants = granted.length > 0 ? granted.join(', ') : 'nothing';
  console.log(`check-native: ${plugin.id} grants ${grants}; checking ${names.join(', ')}`);

  const suppressions = packages.flatMap((entry) => suppressionProblems(entry, plugin.id));
  for (const problem of suppressions) {
    console.error(`error: ${problem}`);
  }

  // Written only when it changes: Clippy checks the crates again after the
  // file changes on disk.
  const folder = `.generated/clippy/${plugin.id}`;
  writeIfChanged(root, {
    path: `${folder}/clippy.toml`,
    text: pluginClippyConfiguration(rootConfiguration, plugin.id, granted),
  });
  const selection = names.flatMap((name) => ['-p', name]);
  const run = spawnSync(
    'cargo',
    ['clippy', '--locked', '--manifest-path', manifest, ...selection, '--lib', '--', '-D', 'warnings'],
    {
      cwd: root,
      encoding: 'utf8',
      env: { ...process.env, CLIPPY_CONF_DIR: join(root, folder) },
      maxBuffer: 1 << 30,
      stdio: ['ignore', 'inherit', 'pipe'],
    },
  );
  process.stderr.write(run.stderr);
  const unresolved = unresolvedPaths(run.stderr);
  if (run.error !== undefined) {
    console.error(`error: could not run cargo: ${run.error.message}`);
  }
  const hasUnresolved = unresolved.length > 0;
  if (hasUnresolved) {
    console.error(
      `error: Clippy cannot resolve ${unresolved.join(', ')} in the configuration of ${plugin.id}; fix the paths in scripts/plugins/lib/native.mjs`,
    );
  }
  const clippyFailed = run.error !== undefined || run.status !== 0;
  const hasSuppressions = suppressions.length > 0;
  if (clippyFailed || hasUnresolved || hasSuppressions) {
    failed.push(plugin.id);
  }
}

if (failed.length > 0) {
  console.error(`check-native: the library crates of ${failed.join(', ')} failed the check`);
  process.exitCode = 1;
} else {
  console.log(
    `check-native: the library crates of ${String(found.plugins.length)} plugins make only the native calls that their manifests grant`,
  );
}
