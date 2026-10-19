// @ts-check
// The steps of `task gen` that `scripts/plugins/generate.mjs` runs (ADR-0035,
// decisions 3 to 6 and 12), as functions that return their results, so that
// generating and checking share them and the tests can call them.

import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';

import { parse } from 'smol-toml';

import { cargoMetadata, explainMetadataFailure, LAUNCHER_FOLDER, lockOf, lockTask, manifestOf } from './cargo.mjs';
import { editorWorkspace, launcherManifest, pluginBindingsCrate, pluginListCrate } from './launcher.mjs';
import { disagreements, readLock } from './lock.mjs';
import { findPlugins, readRules } from './manifests.mjs';
import {
  APP_FOLDER,
  catalogKeys,
  GENERATED_MESSAGES,
  inlangSettings,
  messageSources,
  pluginMessageSource,
  readCatalogs,
} from './messages.mjs';
import { messageBarrel, uiPluginLists, uiPlugins } from './ui.mjs';

/**
 * @typedef {import('./cargo.mjs').Metadata} Metadata
 * @typedef {import('./cargo.mjs').Workspace} Workspace
 * @typedef {import('./launcher.mjs').LauncherPlugin} LauncherPlugin
 * @typedef {import('./lock.mjs').NamedLock} NamedLock
 * @typedef {import('./manifests.mjs').PluginFolder} PluginFolder
 */

/**
 * What the checks of `task gen` found: the plugins with their entry crates,
 * the committed workspaces and their locks.
 * @typedef {object} Inputs
 * @property {NamedLock[]} committedLocks the root's lock first
 * @property {LauncherPlugin[]} launcherPlugins in the order of their starts
 * @property {PluginFolder[]} plugins in the order of their starts
 * @property {Workspace[]} workspaces the root's first
 */

/**
 * A generated file: its path, relative to the root, and its text.
 * @typedef {{ path: string, text: string }} GeneratedFile
 */

/** The desktop binary, a member of the launcher. */
export const BINARY_FOLDER = 'apps/desktop/src-tauri';
/** The desktop app's library, a member of the root workspace. */
export const CORE_FOLDER = 'apps/desktop/core';
/** The launcher's lock, relative to the root. */
export const LAUNCHER_LOCK = `${LAUNCHER_FOLDER}/Cargo.lock`;
/** The fingerprint that the binary's build script checks, relative to the root. */
export const FINGERPRINT_FILE = `${LAUNCHER_FOLDER}/fingerprint`;
/** The launcher as a workspace. */
/** @type {Workspace} */
export const LAUNCHER_WORKSPACE = { folder: LAUNCHER_FOLDER, kind: 'launcher' };

/**
 * A path relative to the root, with forward slashes.
 * @param {string} root
 * @param {string} path absolute
 */
export function fromRoot(root, path) {
  return relative(root, path).replaceAll('\\', '/');
}

/**
 * The folder of a plugin's entry crate in its workspace's metadata, which
 * must be a library of the workspace.
 * @param {string} root
 * @param {Metadata} metadata
 * @param {string} entry the package's name from the manifest
 * @returns {string | null} the crate's folder, relative to the root
 */
export function entryCrateFolder(root, metadata, entry) {
  const members = new Set(metadata.workspace_members);
  const found = metadata.packages.find((candidate) => {
    const isEntry = candidate.name === entry;
    const isMember = members.has(candidate.id);
    const isLibrary = candidate.targets.some((target) => target.kind.includes('lib'));
    return isEntry && isMember && isLibrary;
  });
  return found === undefined ? null : fromRoot(root, dirname(found.manifest_path));
}

/**
 * Checks the plugins' manifests and workspaces, each committed lock against
 * its workspace and the committed locks' agreement, and finds each plugin's
 * entry crate.
 * @param {string} root the repository's root
 * @returns {{ inputs: Inputs, problems: [] } | { inputs: null, problems: string[] }}
 */
export function readInputs(root) {
  const found = findPlugins(root, readRules(root));
  if (found.problems.length > 0) {
    return { inputs: null, problems: found.problems };
  }
  /** @type {Workspace[]} */
  const workspaces = [
    { folder: '.', kind: 'root' },
    ...found.plugins.map((plugin) => ({
      folder: plugin.folder,
      kind: /** @type {const} */ (plugin.validation ? 'fixture' : 'plugin'),
      plugin: plugin.id,
    })),
  ];
  const missingLocks = workspaces.filter((workspace) => !existsSync(join(root, lockOf(workspace))));
  if (missingLocks.length > 0) {
    return {
      inputs: null,
      problems: missingLocks.map(
        (workspace) => `${workspace.folder} has no Cargo.lock. Run \`${lockTask(workspace.plugin)}\` to create it.`,
      ),
    };
  }
  /** @type {NamedLock[]} */
  const committedLocks = workspaces.map((workspace) => ({
    lock: readLock(join(root, lockOf(workspace))),
    name: lockOf(workspace),
  }));
  /** @type {string[]} */
  const problems = [];
  /** @type {Map<string, Metadata>} */
  const metadataByFolder = new Map();
  for (const workspace of workspaces) {
    const result = cargoMetadata(root, manifestOf(workspace), { locked: true, offline: true });
    if (result.ok) {
      metadataByFolder.set(workspace.folder, result.metadata);
    } else {
      problems.push(explainMetadataFailure(workspace, result.stderr, committedLocks));
    }
  }
  /** @type {LauncherPlugin[]} */
  const launcherPlugins = [];
  for (const plugin of found.plugins) {
    const metadata = metadataByFolder.get(plugin.folder);
    if (metadata === undefined) {
      continue;
    }
    const crateFolder = entryCrateFolder(root, metadata, plugin.manifest.rust.entry);
    if (crateFolder === null) {
      problems.push(
        `${plugin.folder}/plugin.toml: the entry crate ${plugin.manifest.rust.entry} is not a library of the plugin's workspace`,
      );
      continue;
    }
    launcherPlugins.push({
      crateFolder,
      entry: plugin.manifest.rust.entry,
      folder: plugin.folder,
      id: plugin.id,
      validation: plugin.validation,
    });
  }
  if (problems.length > 0) {
    return { inputs: null, problems };
  }
  const disagreeing = disagreements(committedLocks).map(
    (crate) =>
      `the committed locks pin ${crate.name} ${crate.range} at different versions (${crate.pins
        .map((pin) => `${pin.version} in ${pin.lock}`)
        .join(', ')}). Run \`task deps:align\`, which brings the plugins' locks to the root's versions.`,
  );
  if (disagreeing.length > 0) {
    return { inputs: null, problems: disagreeing };
  }
  return { inputs: { committedLocks, launcherPlugins, plugins: found.plugins, workspaces }, problems: [] };
}

/**
 * The files that `task gen` writes from its inputs, apart from the
 * launcher's lock and the fingerprint: the inlang projects, the UI's plugin
 * lists and the plugins' message barrels, the launcher's workspace, the
 * plugin list crate, the plugin bindings program, the index of the plugins
 * and the editor workspace.
 * @param {string} root the repository's root
 * @param {Inputs} inputs
 * @returns {GeneratedFile[]}
 */
export function generatedFiles(root, inputs) {
  const committed = JSON.parse(readFileSync(join(root, APP_FOLDER, 'project.inlang', 'settings.json'), 'utf8'));
  const rootManifest = parse(readFileSync(join(root, 'Cargo.toml'), 'utf8'));
  const pluginList = pluginListCrate(inputs.launcherPlugins);
  const pluginBindings = pluginBindingsCrate(inputs.launcherPlugins);
  const index = inputs.plugins.map((plugin) => ({
    crates: plugin.manifest.crates ?? {},
    entry: plugin.manifest.rust.entry,
    folder: plugin.folder,
    id: plugin.id,
    order: plugin.manifest.order,
    secrets: plugin.manifest.secrets ?? {},
    services: plugin.manifest.services ?? {},
    validation: plugin.validation,
  }));
  const editorManifests = [...inputs.workspaces.map(manifestOf), manifestOf(LAUNCHER_WORKSPACE)];
  return [
    ...messageProjects(root, inputs, committed),
    ...uiPluginLists(uiPlugins(root, inputs.plugins)),
    ...inputs.plugins.flatMap((plugin) => {
      const base = readCatalogs(root, pluginMessageSource(plugin)).get(committed.baseLocale) ?? {};
      return messageBarrel(plugin.folder, catalogKeys(base));
    }),
    {
      path: `${LAUNCHER_FOLDER}/Cargo.toml`,
      text: launcherManifest({
        binaryFolder: BINARY_FOLDER,
        coreFolder: CORE_FOLDER,
        launcherFolder: LAUNCHER_FOLDER,
        plugins: inputs.launcherPlugins,
        rootManifest,
      }),
    },
    { path: `${LAUNCHER_FOLDER}/plugin-list/Cargo.toml`, text: pluginList.cargoToml },
    { path: `${LAUNCHER_FOLDER}/plugin-list/src/lib.rs`, text: pluginList.libRs },
    { path: `${LAUNCHER_FOLDER}/plugin-bindings/Cargo.toml`, text: pluginBindings.cargoToml },
    { path: `${LAUNCHER_FOLDER}/plugin-bindings/src/main.rs`, text: pluginBindings.mainRs },
    { path: `${LAUNCHER_FOLDER}/plugins.json`, text: `${JSON.stringify(index, null, 2)}\n` },
    { path: '.generated/mannie.code-workspace', text: editorWorkspace(root, editorManifests) },
  ];
}

/**
 * The inlang projects of the messages (ADR-0035, decision 8): the project
 * that Paraglide compiles and `inlang check` checks, with the core's
 * catalogs and every plugin's and with the committed project's settings and
 * Paraglide options, and for translation tools one project for each source
 * that ships, since they write a whole catalog into every file that a
 * project lists. Those have no Paraglide options, so none of them compiles
 * over the app's catalog. The compiled catalog holds the fixture plugins'
 * messages too, so that the type checks and the unit tests see the fixture
 * plugins' UI; the builds that ship bundle only the messages that their code
 * calls, and their plugin list leaves the fixture plugins out.
 * @param {string} root
 * @param {Inputs} inputs
 * @param {Record<string, unknown>} committed the committed project's settings
 * @returns {GeneratedFile[]}
 */
function messageProjects(root, inputs, committed) {
  const paraglide = readFileSync(join(root, APP_FOLDER, 'project.inlang', 'paraglide.config.js'), 'utf8');
  const sources = messageSources(inputs.plugins);
  const shipped = messageSources(inputs.plugins.filter((plugin) => !plugin.validation));
  /**
   * A project's settings in a folder.
   * @param {string} parent the folder that holds `project.inlang`, relative to the root
   * @param {import('./messages.mjs').MessageSource[]} listed
   * @returns {GeneratedFile}
   */
  const settings = (parent, listed) => ({
    path: `${parent}/project.inlang/settings.json`,
    text: `${JSON.stringify(inlangSettings(committed, listed, parent), null, 2)}\n`,
  });
  return [
    settings(GENERATED_MESSAGES, sources),
    { path: `${GENERATED_MESSAGES}/project.inlang/paraglide.config.js`, text: paraglide },
    ...shipped.map((source) => settings(`${GENERATED_MESSAGES}/inlang/${source.name}`, [source])),
  ];
}

/**
 * Writes a file when its content differs from what is on disk, so that a
 * build that watches it does not start again for nothing.
 * @param {string} root
 * @param {GeneratedFile} file
 * @returns {boolean} whether it wrote
 */
export function writeIfChanged(root, file) {
  const path = join(root, file.path);
  if (isCurrent(root, file)) {
    return false;
  }
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, file.text);
  return true;
}

/**
 * Whether a generated file on disk has the text that `task gen` would write.
 * @param {string} root
 * @param {GeneratedFile} file
 */
export function isCurrent(root, file) {
  const path = join(root, file.path);
  if (!existsSync(path)) {
    return false;
  }
  return readFileSync(path, 'utf8') === file.text;
}
