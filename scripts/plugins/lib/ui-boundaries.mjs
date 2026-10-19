// @ts-check
// The boundaries between the core's UI and the plugins' UI as Vite resolves
// imports (ADR-0035, decision 6): `vite dev`, `vite build` and Vitest refuse
// an import of a plugin's hand-written UI file that leaves the plugin's
// folder other than through the SDK, an import of a plugin's file by a core
// file other than the generated plugin lists, and the transport in any file
// but the generated bindings. ESLint checks the same by folder
// (`mannie/plugin-boundaries`); this check also holds where no lint runs.
//
// The SDK's aliases (apps/desktop/vite.config.ts) run before this check, so
// an import of `@mannie/plugin-sdk/...` arrives as the absolute path of the
// SDK's module, of an installed shadcn-svelte component or of an icon. Tests
// and generated files are not checked here. The validation harness, which
// only validation builds load, uses the transport to call a command that no
// plugin declares (V1), as its lint exception says.

import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';

/**
 * Where the boundaries lie.
 * @typedef {object} UiBoundaries
 * @property {string} app the desktop app's folder, absolute
 * @property {string[]} plugins every plugin's folder, the fixture plugins' included, absolute
 */

/**
 * Whether `path` is `folder` or inside it.
 * @param {string} folder absolute
 * @param {string} path absolute
 */
function isWithin(folder, path) {
  const within = relative(folder, path);
  const leaves = within === '..' || within.startsWith(`..${sep}`);
  return !leaves && !isAbsolute(within);
}

/**
 * The file that a path names without its extension, so that an import of
 * `transport.js` matches `transport.ts`.
 * @param {string} path
 */
function withoutExtension(path) {
  return path.replace(/\.(?:[cm]?[jt]s)$/, '');
}

/**
 * The absolute path that an import names, or `null` for a package or a
 * virtual module.
 * @param {UiBoundaries} boundaries
 * @param {string} source
 * @param {string} importer absolute
 * @returns {string | null}
 */
function importedPath(boundaries, source, importer) {
  if (source.startsWith('#lib/')) {
    return resolve(boundaries.app, 'src', 'lib', source.slice('#lib/'.length));
  }
  const isRelative = source.startsWith('./') || source.startsWith('../');
  if (isRelative) {
    return resolve(dirname(importer), source);
  }
  return isAbsolute(source) ? resolve(source) : null;
}

/**
 * Why Vite refuses an import, or `null` when the boundaries allow it.
 * @param {UiBoundaries} boundaries
 * @param {string} source the specifier, after the SDK's aliases, without a query
 * @param {string} importer the importing file, absolute, without a query
 * @returns {string | null}
 */
export function importRefusal(boundaries, source, importer) {
  const isVirtual = source.startsWith('\0') || source.startsWith('/@') || source.startsWith('virtual:');
  if (isVirtual) {
    return null;
  }
  const lib = join(boundaries.app, 'src', 'lib');
  const sdk = join(lib, 'sdk');
  const transport = join(sdk, 'transport');
  const target = importedPath(boundaries, source, resolve(importer));
  const plugin = boundaries.plugins.find((folder) => isWithin(folder, resolve(importer)));
  const isGenerated =
    plugin === undefined
      ? isWithin(join(lib, 'generated'), resolve(importer))
      : isWithin(join(plugin, 'ui', 'generated'), resolve(importer));
  if (isGenerated) {
    return null;
  }
  const isTransport = target !== null && withoutExtension(target) === transport;
  const isHarness = isWithin(join(lib, 'validation'), resolve(importer));
  if (isTransport && !isHarness) {
    return `${importer} imports the transport, which only a plugin's generated bindings use: call the plugin's commands through them (ADR-0034, decision 9).`;
  }
  if (plugin === undefined) {
    const isCoreFile = isWithin(join(boundaries.app, 'src'), resolve(importer));
    const reachedPlugin = target === null ? undefined : boundaries.plugins.find((folder) => isWithin(folder, target));
    const reachesPlugin = reachedPlugin !== undefined;
    if (isCoreFile && reachesPlugin) {
      return `${importer} imports ${source}, a file of the plugin in ${reachedPlugin}: the core's UI reaches the plugins only through the generated plugin list, #plugin-list (ADR-0035, decision 7).`;
    }
    return null;
  }
  if (isWithin(join(plugin, 'tests'), resolve(importer))) {
    return null;
  }
  return pluginImportRefusal(boundaries, { plugin, source, target }, importer);
}

/**
 * Why Vite refuses an import of a plugin's hand-written UI file, or `null`.
 * @param {UiBoundaries} boundaries
 * @param {{ plugin: string, source: string, target: string | null }} imported
 * @param {string} importer
 * @returns {string | null}
 */
function pluginImportRefusal(boundaries, imported, importer) {
  const { plugin, source, target } = imported;
  const refusal = `${importer} imports ${source}, but a plugin's UI imports only the SDK (@mannie/plugin-sdk), its own files and its generated files (ADR-0034, decision 11).`;
  if (target === null) {
    const isSvelte = source === 'svelte' || source.startsWith('svelte/');
    return isSvelte ? null : refusal;
  }
  if (isWithin(plugin, target)) {
    return null;
  }
  // Only the SDK's aliases name an absolute path: its modules, the installed
  // shadcn-svelte components and the icons.
  const isAliased = isAbsolute(source);
  const sdkTargets = [
    join(boundaries.app, 'src', 'lib', 'sdk'),
    join(boundaries.app, 'src', 'lib', 'components', 'ui'),
    join(boundaries.app, 'node_modules', '@lucide', 'svelte', 'dist', 'icons'),
  ];
  const isSdkTarget = sdkTargets.some((folder) => isWithin(folder, target));
  return isAliased && isSdkTarget ? null : refusal;
}

/**
 * The Vite plugin that refuses what `importRefusal` refuses. It runs before
 * Vite's own resolver and resolves nothing itself.
 * @param {UiBoundaries} boundaries
 */
export function uiBoundariesPlugin(boundaries) {
  return {
    enforce: /** @type {const} */ ('pre'),
    name: 'mannie:plugin-boundaries',
    /**
     * @param {string} source
     * @param {string | undefined} importer
     * @returns {null}
     */
    resolveId(source, importer) {
      if (importer === undefined) {
        return null;
      }
      const refusal = importRefusal(boundaries, source.split('?')[0] ?? source, importer.split('?')[0] ?? importer);
      if (refusal !== null) {
        throw new Error(refusal);
      }
      return null;
    },
  };
}
