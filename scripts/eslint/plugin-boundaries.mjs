// @ts-check
// mannie/plugin-boundaries: the boundaries between the core's files and the
// plugins' files, by folder (ADR-0035, decision 6; ADR-0034, decision 11).
//
// A plugin's files live in its folder, `plugins/<id>/`, or for a fixture
// plugin `tests/fixtures/plugins/<id>/`. Its UI, in `ui/`, imports only the
// SDK, `@mannie/plugin-sdk` and its parts, Svelte, and its own files, its
// generated files included: never Tauri, `#lib`, `$app`, a package or
// another plugin. Its tests, in `tests/`, import packages and the shared
// end-to-end support, `#e2e/<module>.ts`, but no file of the core or of
// another plugin. The core's files never import a plugin's file, so the
// core's UI reaches the plugins only through the generated plugin list,
// except that the core's tests use the fixture plugins. Nor do they name a
// plugin's message keys, `m.<prefix>_...`, since a plugin words its own
// text (ADR-0035, decision 8).
//
// The `plugins` option lists every plugin with its folder, as an absolute
// path, its ID, its prefix and whether it is a fixture plugin.
import { dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';

/**
 * A plugin as the rule knows it.
 * @typedef {object} BoundaryPlugin
 * @property {boolean} fixture whether it is a fixture plugin, which the core's tests may use
 * @property {string} folder absolute
 * @property {string} id
 * @property {string} prefix the prefix of its message keys, without the `_`
 */

/** The SDK's name and the start of its parts. */
const SDK = '@mannie/plugin-sdk';

/** The aliases of the core's `src/lib/` and SvelteKit's modules, which a plugin never imports. */
const CORE_PREFIXES = ['#lib/', '$lib/', '$app/'];

/**
 * Whether `path` is inside `folder`.
 * @param {string} folder
 * @param {string} path absolute
 */
function isWithin(folder, path) {
  const within = relative(folder, path);
  const leaves = within === '' || within === '..' || within.startsWith(`..${sep}`);
  return !leaves && !isAbsolute(within);
}

/**
 * The absolute path that a relative import names, or `null` for a package or an alias.
 * @param {string} filename the importing file
 * @param {string} source
 */
function relativeTarget(filename, source) {
  const isRelative = source.startsWith('./') || source.startsWith('../');
  return isRelative ? resolve(dirname(filename), source) : null;
}

/**
 * Whether a plugin's UI may import a package or an alias by its name.
 * @param {string} source
 */
function isAllowedName(source) {
  const isSdk = source === SDK || source.startsWith(`${SDK}/`);
  const isSvelte = source === 'svelte' || source.startsWith('svelte/');
  return isSdk || isSvelte;
}

/** @type {import('eslint').Rule.RuleModule} */
export const pluginBoundaries = {
  meta: {
    type: 'problem',
    docs: {
      description: "Keep the core's files and the plugins' files apart, by folder",
    },
    messages: {
      core: "The core's files never import the {{plugin}} plugin's files: the core's UI reaches the plugins only through the generated plugin list, #plugin-list (ADR-0035, decision 7).",
      messageKey:
        'The core never names a message of the {{plugin}} plugin, here {{key}}: the plugin words its own text (ADR-0035, decision 8).',
      otherPlugin:
        "A plugin's files never import another plugin's files, here the {{plugin}} plugin's (ADR-0030, decision 10).",
      pluginTestImport:
        "A plugin's tests import packages, #e2e/<module>.ts and the plugin's own files, never the core's files: {{source}} (ADR-0035, decision 9).",
      pluginUiImport:
        "A plugin's UI imports only the SDK (@mannie/plugin-sdk), svelte and its own files, its generated files included: {{source}} is none of these (ADR-0034, decision 11).",
    },
    schema: [
      {
        additionalProperties: false,
        properties: {
          plugins: {
            items: {
              additionalProperties: false,
              properties: {
                fixture: { type: 'boolean' },
                folder: { type: 'string' },
                id: { type: 'string' },
                prefix: { type: 'string' },
              },
              required: ['fixture', 'folder', 'id', 'prefix'],
              type: 'object',
            },
            type: 'array',
          },
          testsFolder: { type: 'string' },
        },
        required: ['plugins', 'testsFolder'],
        type: 'object',
      },
    ],
  },
  create(context) {
    const [{ plugins, testsFolder }] = /** @type {[{ plugins: BoundaryPlugin[]; testsFolder: string }]} */ (
      context.options
    );
    const filename = resolve(context.filename);
    const owner = plugins.find((plugin) => isWithin(plugin.folder, filename));
    const isCoreFile = owner === undefined;
    const isPluginTest = owner !== undefined && isWithin(join(owner.folder, 'tests'), filename);
    const isCoreTest = isCoreFile && isWithin(testsFolder, filename);

    /**
     * @param {import('estree').Node} node
     * @param {string} source
     */
    function checkPluginImport(node, source) {
      if (owner === undefined) {
        return;
      }
      const target = relativeTarget(filename, source);
      if (target !== null) {
        if (isWithin(owner.folder, target)) {
          return;
        }
        const other = plugins.find((plugin) => isWithin(plugin.folder, target));
        if (other !== undefined) {
          context.report({ data: { plugin: other.id }, messageId: 'otherPlugin', node });
          return;
        }
        context.report({ data: { source }, messageId: isPluginTest ? 'pluginTestImport' : 'pluginUiImport', node });
        return;
      }
      if (isPluginTest) {
        const namesCore = CORE_PREFIXES.some((prefix) => source.startsWith(prefix));
        if (namesCore) {
          context.report({ data: { source }, messageId: 'pluginTestImport', node });
        }
        return;
      }
      if (!isAllowedName(source)) {
        context.report({ data: { source }, messageId: 'pluginUiImport', node });
      }
    }

    /**
     * @param {import('estree').Node} node
     * @param {string} source
     */
    function checkCoreImport(node, source) {
      const target = relativeTarget(filename, source);
      if (target === null) {
        return;
      }
      const reached = plugins.find((plugin) => isWithin(plugin.folder, target));
      if (reached === undefined) {
        return;
      }
      const isFixtureForTest = reached.fixture && isCoreTest;
      if (!isFixtureForTest) {
        context.report({ data: { plugin: reached.id }, messageId: 'core', node });
      }
    }

    /**
     * @param {import('estree').Node} node
     * @param {unknown} source
     */
    function check(node, source) {
      if (typeof source !== 'string') {
        return;
      }
      if (owner === undefined) {
        checkCoreImport(node, source);
      } else {
        checkPluginImport(node, source);
      }
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
      MemberExpression(node) {
        const isCatalog = node.object.type === 'Identifier' && node.object.name === 'm';
        const isNamed = node.property.type === 'Identifier' && !node.computed;
        if (!isCoreFile || !isCatalog || !isNamed) {
          return;
        }
        const key = /** @type {import('estree').Identifier} */ (node.property).name;
        const plugin = plugins.find((candidate) => key.startsWith(`${candidate.prefix}_`));
        if (plugin !== undefined) {
          context.report({ data: { key, plugin: plugin.id }, messageId: 'messageKey', node });
        }
      },
    };
  },
};
