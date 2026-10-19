// @ts-check
// The dependency rules between the core and the plugins (ADR-0034, decision
// 2; ADR-0035, decision 6), over the resolve graphs of `cargo metadata`,
// which hold every dependency kind for every target:
//
// - no crate of the core depends on a crate in a plugin's folder, and in the
//   launcher only the generated crates do;
// - a plugin's crates depend only on the crates of its own folder, on the
//   plugin SDK and on crates from a registry. Their tests may also use the
//   plugin harness: a dev-dependency may, and so may a crate of the plugin
//   that only tests reach. The plugin's programs, the crates that its
//   manifest lists under them, and the crates that only tests reach may also
//   use the host kit. What the plugin's entry crate ships, its graph without
//   dev-dependencies, reaches neither (ADR-0034, decision 2);
// - no plugin depends on another plugin's crates;
// - a crate that a plugin's manifest lists under `[crates] exclusive` is a
//   direct dependency only of the plugin's crates and of the other crates of
//   that list, and no two plugins list one crate;
// - the SDK re-exports nothing with a glob, so its surface stays explicit.

import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join, relative } from 'node:path';

import {
  cargoMetadata,
  committedWorkspaces,
  explainMetadataFailure,
  LAUNCHER_FOLDER,
  lockOf,
  manifestOf,
} from './cargo.mjs';
import { readLock } from './lock.mjs';
import { compareText, findPlugins, readRules } from './manifests.mjs';

/**
 * @typedef {import('./cargo.mjs').Metadata} Metadata
 * @typedef {import('./cargo.mjs').MetadataPackage} MetadataPackage
 */

/**
 * A plugin as the boundary check sees it.
 * @typedef {object} BoundaryPlugin
 * @property {string} entry its entry crate, which the desktop app links
 * @property {string[]} exclusive the registry crates that only this plugin may use
 * @property {string} folder its folder, relative to the root, with forward slashes
 * @property {string} id
 * @property {string[]} programs its programs' packages and the library crates that only they use
 */

/**
 * The folders of the plugin SDK's crates, which plugins may depend on.
 */
export const SDK_CRATE_FOLDERS = ['crates/mannie-plugin-sdk'];

/** The plugin harness, which plugins' tests may use (ADR-0034, decision 1). */
export const HARNESS_FOLDER = 'crates/mannie-plugin-harness';

/** The host kit, which plugins' programs may use (ADR-0034, decision 12). */
export const HOST_KIT_FOLDER = 'crates/mannie-host-kit';

/**
 * Where a package belongs: a plugin's folder, the launcher's generated
 * crates, the core (every other path package) or a registry.
 * @param {MetadataPackage} entry
 * @param {string} root the repository's root
 * @param {BoundaryPlugin[]} plugins
 * @param {string} launcherFolder relative to the root
 * @returns {{ kind: 'core' | 'generated' | 'harness' | 'host-kit' | 'registry' | 'sdk' } | { kind: 'plugin', plugin: BoundaryPlugin }}
 */
export function ownerOf(entry, root, plugins, launcherFolder) {
  if (entry.source !== null) {
    return { kind: 'registry' };
  }
  const folder = relative(root, entry.manifest_path)
    .replaceAll('\\', '/')
    .replace(/\/Cargo\.toml$/u, '');
  const plugin = plugins.find((candidate) => `${folder}/`.startsWith(`${candidate.folder}/`));
  if (plugin !== undefined) {
    return { kind: 'plugin', plugin };
  }
  if (folder.startsWith(`${launcherFolder}/`)) {
    return { kind: 'generated' };
  }
  if (SDK_CRATE_FOLDERS.includes(folder)) {
    return { kind: 'sdk' };
  }
  if (folder === HARNESS_FOLDER) {
    return { kind: 'harness' };
  }
  return { kind: folder === HOST_KIT_FOLDER ? 'host-kit' : 'core' };
}

/**
 * Whether a dependency edge is a dev-dependency on every target, which only
 * tests, examples and benchmarks build.
 * @param {{ dep_kinds: { kind: string | null }[] }} dependency
 */
function isDevOnly(dependency) {
  return dependency.dep_kinds.every((kind) => kind.kind === 'dev');
}

/**
 * The packages that a graph reaches from `starts` over edges that are not
 * dev-only, by their IDs, the starts included.
 * @param {Metadata} metadata
 * @param {string[]} starts the IDs of the packages to start from
 * @param {(id: string) => boolean} follows whether the walk goes on through a package
 * @returns {Set<string>}
 */
function shippedFrom(metadata, starts, follows) {
  const nodes = new Map((metadata.resolve?.nodes ?? []).map((node) => [node.id, node]));
  const reached = new Set(starts);
  const queue = [...starts];
  while (queue.length > 0) {
    const node = nodes.get(String(queue.pop()));
    for (const dependency of node?.deps ?? []) {
      const isShipped = !isDevOnly(dependency);
      const isNew = !reached.has(dependency.pkg);
      if (isShipped && isNew && follows(dependency.pkg)) {
        reached.add(dependency.pkg);
        queue.push(dependency.pkg);
      }
    }
  }
  return reached;
}

/**
 * The problem of one dependency edge, or `null` for an edge that the rules
 * allow.
 * @param {object} edge
 * @param {string} edge.edge `<from> -> <to>`
 * @param {Map<string, BoundaryPlugin>} edge.exclusiveOwner the plugin that lists each exclusive crate
 * @param {MetadataPackage} edge.from
 * @param {ReturnType<typeof ownerOf>} edge.fromOwner
 * @param {boolean} edge.isDev whether the edge is a dev-dependency on every target
 * @param {boolean} edge.isTestOnly whether only tests reach the crate that the edge starts from
 * @param {MetadataPackage} edge.to
 * @param {ReturnType<typeof ownerOf>} edge.toOwner
 * @returns {string | null}
 */
function edgeProblem({ edge, exclusiveOwner, from, fromOwner, isDev, isTestOnly, to, toOwner }) {
  if (toOwner.kind === 'registry') {
    const owner = exclusiveOwner.get(to.name);
    if (owner === undefined) {
      return null;
    }
    const isOwnersCrate = fromOwner.kind === 'plugin' && fromOwner.plugin.id === owner.id;
    const isListedWithIt = fromOwner.kind === 'registry' && owner.exclusive.includes(from.name);
    if (isOwnersCrate || isListedWithIt) {
      return null;
    }
    return `${edge}, but only the plugin ${owner.id} may use ${to.name}, which its manifest lists under [crates] exclusive`;
  }
  if (fromOwner.kind === 'plugin') {
    if (toOwner.kind === 'plugin') {
      const isOwnPlugin = fromOwner.plugin.id === toOwner.plugin.id;
      return isOwnPlugin
        ? null
        : `${edge}: the plugin ${fromOwner.plugin.id} depends on the plugin ${toOwner.plugin.id}`;
    }
    const isForTests = isDev || isTestOnly;
    if (toOwner.kind === 'harness') {
      return isForTests
        ? null
        : `${edge}: only the tests of the plugin ${fromOwner.plugin.id} may use the plugin harness, as a dev-dependency or from a crate that only tests reach (ADR-0034, decision 1)`;
    }
    if (toOwner.kind === 'host-kit') {
      const isProgramCrate = fromOwner.plugin.programs.includes(from.name);
      return isForTests || isProgramCrate
        ? null
        : `${edge}: only the programs of the plugin ${fromOwner.plugin.id}, and the crates that its manifest lists under them, may use the host kit (ADR-0034, decision 12)`;
    }
    if (toOwner.kind !== 'core') {
      return null;
    }
    return `${edge}: the plugin ${fromOwner.plugin.id} reaches the core only through the plugin SDK (ADR-0034, decision 2)`;
  }
  const isPluginCrate = toOwner.kind === 'plugin';
  const isGenerated = fromOwner.kind === 'generated';
  if (isPluginCrate && !isGenerated) {
    return `${edge}: the core depends on the plugin ${toOwner.plugin.id}`;
  }
  return null;
}

/**
 * The problems that the dependency edges of the graphs have.
 * @param {object} options
 * @param {{ metadata: Metadata, name: string }[]} options.graphs every workspace's metadata and the launcher's
 * @param {string} options.launcherFolder relative to the root
 * @param {BoundaryPlugin[]} options.plugins
 * @param {string} options.root the repository's root
 * @returns {string[]}
 */
export function boundaryProblems({ graphs, launcherFolder, plugins, root }) {
  /** @type {Set<string>} */
  const problems = new Set();
  /** @type {Map<string, BoundaryPlugin>} */
  const exclusiveOwner = new Map();
  for (const plugin of plugins) {
    for (const crate of plugin.exclusive) {
      const other = exclusiveOwner.get(crate);
      if (other !== undefined) {
        problems.add(`the plugins ${other.id} and ${plugin.id} both list ${crate} under [crates] exclusive`);
      }
      exclusiveOwner.set(crate, plugin);
    }
  }
  for (const { metadata, name } of graphs) {
    const packages = new Map(metadata.packages.map((entry) => [entry.id, entry]));
    const owners = new Map(metadata.packages.map((entry) => [entry.id, ownerOf(entry, root, plugins, launcherFolder)]));
    const shipped = shippedPluginCrates(metadata, plugins, owners);
    for (const problem of shippedEntryProblems(metadata, plugins, owners)) {
      problems.add(`${name}: ${problem}`);
    }
    for (const node of metadata.resolve?.nodes ?? []) {
      const from = packages.get(node.id);
      const fromOwner = owners.get(node.id);
      if (from === undefined) {
        continue;
      }
      if (fromOwner === undefined) {
        continue;
      }
      const isTestOnly = fromOwner.kind === 'plugin' && !shipped.has(node.id);
      for (const dependency of node.deps) {
        const to = packages.get(dependency.pkg);
        const toOwner = owners.get(dependency.pkg);
        if (to === undefined) {
          continue;
        }
        if (toOwner === undefined) {
          continue;
        }
        const problem = edgeProblem({
          edge: `${from.name} -> ${to.name}`,
          exclusiveOwner,
          from,
          fromOwner,
          isDev: isDevOnly(dependency),
          isTestOnly,
          to,
          toOwner,
        });
        if (problem !== null) {
          problems.add(`${name}: ${problem}`);
        }
      }
    }
  }
  return [...problems].sort();
}

/**
 * Whether a package's owner is the plugin `plugin`.
 * @param {ReturnType<typeof ownerOf> | undefined} owner
 * @param {BoundaryPlugin} plugin
 */
function isOwnedBy(owner, plugin) {
  if (owner?.kind !== 'plugin') {
    return false;
  }
  return owner.plugin.id === plugin.id;
}

/**
 * The plugins' crates that a graph ships: those that a plugin's entry crate
 * or its programs reach over edges that are not dev-only, within the
 * plugin's own crates. Every other crate of a plugin is reached only by its
 * tests.
 * @param {Metadata} metadata
 * @param {BoundaryPlugin[]} plugins
 * @param {Map<string, ReturnType<typeof ownerOf>>} owners the owner of each package, by its ID
 * @returns {Set<string>}
 */
function shippedPluginCrates(metadata, plugins, owners) {
  /** @type {Set<string>} */
  const shipped = new Set();
  for (const plugin of plugins) {
    const isOwn = (/** @type {string} */ id) => isOwnedBy(owners.get(id), plugin);
    const isRoot = (/** @type {MetadataPackage} */ entry) => {
      const isEntry = entry.name === plugin.entry;
      return isEntry || plugin.programs.includes(entry.name);
    };
    const roots = metadata.packages
      .filter((entry) => isOwn(entry.id))
      .filter(isRoot)
      .map((entry) => entry.id);
    for (const id of shippedFrom(metadata, roots, isOwn)) {
      shipped.add(id);
    }
  }
  return shipped;
}

/**
 * The library crates that a plugin's entry crate ships: the entry crate and
 * the plugin's crates that it reaches over edges that are not dev-only.
 * Clippy's rule against direct native calls covers these (ADR-0035,
 * decision 6). The plugin's programs, the crates that only they use and the
 * crates that only tests reach are not among them.
 * @param {Metadata} metadata the plugin workspace's
 * @param {BoundaryPlugin} plugin
 * @param {string} root the repository's root
 * @returns {MetadataPackage[]} by name
 */
export function shippedLibraryPackages(metadata, plugin, root) {
  const owners = new Map(metadata.packages.map((entry) => [entry.id, ownerOf(entry, root, [plugin], LAUNCHER_FOLDER)]));
  const isOwn = (/** @type {string} */ id) => isOwnedBy(owners.get(id), plugin);
  const entry = metadata.packages.find((candidate) => {
    const isEntry = candidate.name === plugin.entry;
    return isEntry && isOwn(candidate.id);
  });
  if (entry === undefined) {
    return [];
  }
  const shipped = shippedFrom(metadata, [entry.id], isOwn);
  return metadata.packages.filter((candidate) => shipped.has(candidate.id)).sort((a, b) => compareText(a.name, b.name));
}

/**
 * Why a plugin's entry crate ships the harness or the host kit: its graph
 * without dev-dependencies reaches one of them.
 * @param {Metadata} metadata
 * @param {BoundaryPlugin[]} plugins
 * @param {Map<string, ReturnType<typeof ownerOf>>} owners the owner of each package, by its ID
 * @returns {string[]}
 */
function shippedEntryProblems(metadata, plugins, owners) {
  /** @type {string[]} */
  const problems = [];
  for (const plugin of plugins) {
    const entry = metadata.packages.find((candidate) => {
      const isOwn = isOwnedBy(owners.get(candidate.id), plugin);
      const isEntry = candidate.name === plugin.entry;
      return isOwn && isEntry;
    });
    if (entry === undefined) {
      continue;
    }
    for (const id of shippedFrom(metadata, [entry.id], () => true)) {
      const kind = owners.get(id)?.kind;
      const isTestTool = kind === 'harness' || kind === 'host-kit';
      if (isTestTool) {
        problems.push(
          `the entry crate ${plugin.entry} of the plugin ${plugin.id} ships ${kind === 'harness' ? 'the plugin harness' : 'the host kit'}, which only its ${kind === 'harness' ? 'tests' : 'programs'} may use (ADR-0034, decision 2)`,
        );
      }
    }
  }
  return problems;
}

/**
 * The Rust files of a folder, recursively.
 * @param {string} folder absolute
 * @returns {string[]}
 */
export function rustFiles(folder) {
  return readdirSync(folder, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.endsWith('.rs'))
    .map((entry) => join(entry.parentPath, entry.name));
}

/**
 * The glob re-exports, `pub use ...::*;`, in the SDK's sources.
 * @param {string} root the repository's root
 * @returns {string[]}
 */
export function globReexportProblems(root) {
  /** @type {string[]} */
  const problems = [];
  for (const folder of SDK_CRATE_FOLDERS) {
    for (const file of rustFiles(join(root, folder, 'src'))) {
      const lines = readFileSync(file, 'utf8').split('\n');
      lines.forEach((line, index) => {
        if (/^\s*pub(?:\([^)]*\))?\s+use\s+[^;]*\*/u.test(line)) {
          problems.push(
            `${relative(root, file).replaceAll('\\', '/')}:${String(index + 1)}: the plugin SDK re-exports items one by one, never with a glob`,
          );
        }
      });
    }
  }
  return problems;
}

/**
 * A plugin as the boundary check sees it, from its manifest.
 * @param {import('./manifests.mjs').PluginFolder} plugin
 * @returns {BoundaryPlugin}
 */
export function boundaryPlugin(plugin) {
  const programs = plugin.manifest.programs ?? [];
  return {
    entry: plugin.manifest.rust.entry,
    exclusive: plugin.manifest.crates?.exclusive ?? [],
    folder: plugin.folder,
    id: plugin.id,
    programs: programs.flatMap((program) => [program.package, ...(program.crates ?? [])]),
  };
}

/**
 * Checks the boundaries of the repository: the plugins' manifests, the
 * graphs of every committed workspace and of the launcher once `task gen`
 * wrote it, and the SDK's re-exports.
 * @param {string} root the repository's root
 * @returns {{ graphs: number, problems: string[] }}
 */
export function checkBoundaries(root) {
  const found = findPlugins(root, readRules(root));
  if (found.problems.length > 0) {
    return { graphs: 0, problems: found.problems };
  }
  const plugins = found.plugins.map(boundaryPlugin);
  /** @type {import('./cargo.mjs').Workspace[]} */
  const workspaces = committedWorkspaces(root);
  if (existsSync(join(root, LAUNCHER_FOLDER, 'Cargo.toml'))) {
    workspaces.push({ folder: LAUNCHER_FOLDER, kind: 'launcher' });
  }
  const committed = workspaces
    .filter((workspace) => workspace.kind !== 'launcher')
    .map((workspace) => ({ lock: readLock(join(root, lockOf(workspace))), name: lockOf(workspace) }));
  /** @type {string[]} */
  const problems = [];
  /** @type {{ metadata: Metadata, name: string }[]} */
  const graphs = [];
  for (const workspace of workspaces) {
    const result = cargoMetadata(root, manifestOf(workspace), { locked: true, offline: true });
    if (result.ok) {
      graphs.push({ metadata: result.metadata, name: manifestOf(workspace) });
    } else {
      problems.push(explainMetadataFailure(workspace, result.stderr, committed));
    }
  }
  problems.push(
    ...boundaryProblems({ graphs, launcherFolder: LAUNCHER_FOLDER, plugins, root }),
    ...globReexportProblems(root),
  );
  return { graphs: graphs.length, problems };
}
