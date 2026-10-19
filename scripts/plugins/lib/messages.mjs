// @ts-check
// The message catalogs of the core and of the plugins (ADR-0035, decision 8;
// ADR-0019). The core keeps `apps/desktop/messages/<locale>.json`, and each
// plugin keeps `messages/<locale>.json` in its folder, with every key under
// its prefix. `task gen` writes an inlang project that lists the core's file
// and then each plugin's, in the order of the plugins' starts, so Paraglide
// compiles one catalog for each locale with one `m` namespace, and checks
// what inlang does not: a key outside its file's prefix, a key in two
// sources, a missing name or description of a plugin, and a plugin's catalog
// for a locale that the core does not register.

import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join, posix } from 'node:path';

import { prefixOf } from './manifests.mjs';

/**
 * A source of messages: the core's catalogs or a plugin's.
 * @typedef {object} MessageSource
 * @property {string} folder its catalogs' folder, relative to the root, with forward slashes
 * @property {string} name `core`, or the plugin's ID
 * @property {string | null} prefix the plugin's prefix, or none for the core
 */

/** The desktop app, which compiles the catalogs, relative to the root. */
export const APP_FOLDER = 'apps/desktop';
/** The core's catalogs, relative to the root. */
export const CORE_MESSAGES = `${APP_FOLDER}/messages`;
/** The generated projects, relative to the root. */
export const GENERATED_MESSAGES = `${APP_FOLDER}/.generated`;

/**
 * The sources of messages: the core's, then each plugin's in the order of
 * the plugins' starts. A plugin without a `messages` folder has none, which
 * the checks refuse, since every plugin names itself.
 * @param {{ folder: string, id: string }[]} plugins in the order of their starts
 * @returns {MessageSource[]}
 */
export function messageSources(plugins) {
  return [{ folder: CORE_MESSAGES, name: 'core', prefix: null }, ...plugins.map(pluginMessageSource)];
}

/**
 * A plugin's source of messages: its folder's `messages`, under its prefix.
 * @param {{ folder: string, id: string }} plugin
 * @returns {MessageSource}
 */
export function pluginMessageSource(plugin) {
  return { folder: `${plugin.folder}/messages`, name: plugin.id, prefix: prefixOf(plugin.id) };
}

/**
 * The settings of a generated inlang project: the committed project's, with
 * the module paths moved to the project's folder and one path pattern for
 * each source.
 * @param {Record<string, unknown>} committed the committed `settings.json`, parsed
 * @param {MessageSource[]} sources
 * @param {string} projectParent the folder that holds the generated project, relative to the root
 * @returns {Record<string, unknown>}
 */
export function inlangSettings(committed, sources, projectParent) {
  const modules = /** @type {string[]} */ (committed.modules ?? []);
  const toApp = posix.relative(projectParent, APP_FOLDER);
  return {
    ...committed,
    modules: modules.map((module) => (module.startsWith('./') ? posix.join(toApp, module.slice(2)) : module)),
    'plugin.inlang.messageFormat': {
      .../** @type {Record<string, unknown>} */ (committed['plugin.inlang.messageFormat'] ?? {}),
      pathPattern: sources.map((source) => `${posix.relative(projectParent, source.folder)}/{locale}.json`),
    },
  };
}

/**
 * The keys of a catalog, without `$schema`.
 * @param {Record<string, unknown>} catalog
 */
export function catalogKeys(catalog) {
  return Object.keys(catalog).filter((key) => key !== '$schema');
}

/**
 * The catalogs of a source by locale, read from its folder.
 * @param {string} root
 * @param {MessageSource} source
 * @returns {Map<string, Record<string, unknown>>}
 */
export function readCatalogs(root, source) {
  /** @type {Map<string, Record<string, unknown>>} */
  const catalogs = new Map();
  const folder = join(root, source.folder);
  if (!existsSync(folder)) {
    return catalogs;
  }
  for (const file of readdirSync(folder)
    .filter((name) => name.endsWith('.json'))
    .sort()) {
    catalogs.set(file.slice(0, -'.json'.length), JSON.parse(readFileSync(join(folder, file), 'utf8')));
  }
  return catalogs;
}

/**
 * The problems of the catalogs of every source.
 * @param {MessageSource[]} sources the core's first
 * @param {Map<string, Map<string, Record<string, unknown>>>} catalogs each source's catalogs by locale, by the source's name
 * @param {{ baseLocale: string, locales: string[] }} settings the committed project's locales
 * @returns {string[]}
 */
export function messageProblems(sources, catalogs, settings) {
  /** @type {string[]} */
  const problems = [];
  const pluginPrefixes = sources.flatMap((source) => (source.prefix === null ? [] : [source.prefix]));
  /** @type {Map<string, string>} */
  const owners = new Map();
  for (const source of sources) {
    const byLocale = catalogs.get(source.name) ?? new Map();
    const where = (/** @type {string} */ locale) => `${source.folder}/${locale}.json`;
    for (const [locale, catalog] of byLocale) {
      if (!settings.locales.includes(locale)) {
        problems.push(`${where(locale)}: the core registers no locale ${locale} in project.inlang/settings.json`);
      }
      for (const key of catalogKeys(catalog)) {
        const isPluginKey = source.prefix !== null;
        const isOwnKey = key.startsWith(`${String(source.prefix)}_`);
        if (isPluginKey && !isOwnKey) {
          problems.push(`${where(locale)}: ${key} is outside the plugin's prefix ${String(source.prefix)}_`);
        }
        const plugin = pluginPrefixes.find((prefix) => key.startsWith(`${prefix}_`));
        const isUnderPlugin = plugin !== undefined;
        if (!isPluginKey && isUnderPlugin) {
          problems.push(
            `${where(locale)}: ${key} is under the prefix of the plugin ${String(plugin)}, so it belongs in its catalog`,
          );
        }
        const owner = owners.get(`${locale} ${key}`);
        const isKnown = owner !== undefined;
        const isOtherSource = owner !== source.name;
        if (isKnown && isOtherSource) {
          problems.push(`${where(locale)}: ${key} is also in the catalog of ${String(owner)}`);
        }
        owners.set(`${locale} ${key}`, source.name);
      }
    }
    if (source.prefix !== null) {
      const base = byLocale.get(settings.baseLocale) ?? {};
      for (const required of [`${source.prefix}_plugin_name`, `${source.prefix}_plugin_description`]) {
        if (!Object.hasOwn(base, required)) {
          problems.push(
            `${where(settings.baseLocale)}: the plugin needs the message ${required}, which names it on the Plugins page`,
          );
        }
      }
    }
  }
  return problems;
}
