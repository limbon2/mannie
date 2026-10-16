// @ts-check
// mannie/plugin-boundaries: the core's UI files and other plugins' UI files
// never import a plugin's UI files (ADR-0030, decision 10).
//
// A plugin's UI files stay in the desktop app's folders for each kind, under
// the plugin's ID: `components/app/<id>/`, and the files named `<id>` or
// `<id>-...` in the other folders of `src/lib/`, such as
// `registrations/workers.ts`, `state/workers-actions.svelte.ts` and
// `utils/workers-messages.ts`. The composition root's list of plugins,
// `registrations/plugin-list.ts`, is the one core file that imports a
// plugin's UI, and only its registration. A plugin's UI may import the
// core's. The `libFolder` option is the absolute path of the app's
// `src/lib/`, and `plugins` lists the plugins' IDs, which are the names of
// their folders in plugins/.
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';

/** The subpath import and the alias that name `src/lib/`. */
const LIB_PREFIXES = ['#lib/', '$lib/'];

/** The composition root's list of plugins, relative to `src/lib/`. */
const PLUGIN_LIST = join('registrations', 'plugin-list.ts');

/**
 * Whether a file or folder name belongs to a plugin: the plugin's ID alone,
 * or followed by `-` or `.`.
 *
 * @param {string} name
 * @param {string} plugin
 * @returns {boolean}
 */
function belongsTo(name, plugin) {
  const isId = name === plugin;
  const startsWithId = name.startsWith(`${plugin}-`) || name.startsWith(`${plugin}.`);
  return isId || startsWithId;
}

/**
 * The plugin that owns a file of `src/lib/`, or `null` for a file of the core.
 *
 * @param {string} libFolder
 * @param {string} path An absolute path.
 * @param {readonly string[]} plugins
 * @returns {string | null}
 */
function ownerOf(libFolder, path, plugins) {
  const inLib = relative(libFolder, path);
  const isOutside = inLib === '..' || inLib.startsWith(`..${sep}`) || isAbsolute(inLib);
  if (isOutside) {
    return null;
  }
  const segments = inLib.split(sep);
  const isAppComponent = segments[0] === 'components' && segments[1] === 'app';
  // The plugin's folder of components, or its file in another kind's folder.
  const named = isAppComponent ? segments[2] : segments[1];
  if (named === undefined) {
    return null;
  }
  return plugins.find((plugin) => belongsTo(named, plugin)) ?? null;
}

/**
 * The absolute path that an import names, or `null` for a package.
 *
 * @param {string} libFolder
 * @param {string} filename The importing file.
 * @param {string} source
 * @returns {string | null}
 */
function importedPath(libFolder, filename, source) {
  const prefix = LIB_PREFIXES.find((candidate) => source.startsWith(candidate));
  if (prefix !== undefined) {
    return resolve(libFolder, source.slice(prefix.length));
  }
  const isRelative = source.startsWith('./') || source.startsWith('../');
  return isRelative ? resolve(dirname(filename), source) : null;
}

/** @type {import('eslint').Rule.RuleModule} */
export const pluginBoundaries = {
  meta: {
    type: 'problem',
    docs: {
      description: "Keep a plugin's UI files out of the core's UI and other plugins' UI",
    },
    messages: {
      core: "The core's UI reaches the {{plugin}} plugin's UI only through its registration in registrations/plugin-list.ts (ADR-0030, decision 10).",
      otherPlugin:
        "A plugin's UI never imports another plugin's UI, here the {{plugin}} plugin's (ADR-0030, decision 10).",
    },
    schema: [
      {
        additionalProperties: false,
        properties: {
          libFolder: { type: 'string' },
          plugins: { items: { type: 'string' }, type: 'array' },
        },
        required: ['libFolder', 'plugins'],
        type: 'object',
      },
    ],
  },
  create(context) {
    const [{ libFolder, plugins }] = /** @type {[{ libFolder: string; plugins: string[] }]} */ (context.options);
    const importer = ownerOf(libFolder, context.filename, plugins);
    const isPluginList = relative(libFolder, context.filename) === PLUGIN_LIST;

    /**
     * @param {import('estree').Node} node
     * @param {unknown} source
     */
    function check(node, source) {
      if (typeof source !== 'string') {
        return;
      }
      const target = importedPath(libFolder, context.filename, source);
      if (target === null) {
        return;
      }
      const owner = ownerOf(libFolder, target, plugins);
      const isCore = owner === null;
      const isOwn = owner === importer;
      if (isCore || isOwn) {
        return;
      }
      // TypeScript imports name the registration's `.ts` file as `.js`.
      const namesRegistration = [`${owner}.js`, `${owner}.ts`].some(
        (name) => target === join(libFolder, 'registrations', name),
      );
      if (isPluginList && namesRegistration) {
        return;
      }
      context.report({ data: { plugin: owner }, messageId: importer === null ? 'core' : 'otherPlugin', node });
    }

    return {
      ExportAllDeclaration(node) {
        check(node.source, node.source.value);
      },
      ExportNamedDeclaration(node) {
        if (node.source) {
          check(node.source, node.source.value);
        }
      },
      ImportDeclaration(node) {
        check(node.source, node.source.value);
      },
      ImportExpression(node) {
        if (node.source.type === 'Literal') {
          check(node.source, node.source.value);
        }
      },
    };
  },
};
