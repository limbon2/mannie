// @ts-check
// The plugins' manifests, `plugins/<id>/plugin.toml` (ADR-0035, decision 1),
// and the checks that `task gen` makes on them before any Rust compiles
// (decision 6). The rules come from the plugin SDK: the shape from
// `plugin.schema.json`, and the SDK's major, the reserved IDs, the table
// prefixes, the services and the names of variables that installed programs
// may not inherit from `plugin-rules.json`. The SDK's tests keep both files
// equal to what the host checks with, and
// `crates/mannie-plugin-sdk/tests/fixtures/plugin-ids.json` and
// `plugin-processes.json` hold IDs and names that the host's check and this
// one must judge alike.
//
// Validation builds also bundle the fixture plugins of `tests/fixtures/plugins/`
// (decision 9), which the same checks cover.

import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

import { parse } from 'smol-toml';

import { nativeGrants } from './native.mjs';

/**
 * A JSON Schema, as far as the manifest's schema uses it.
 * @typedef {{ [keyword: string]: unknown }} JsonSchema
 */

/**
 * The rules of manifests, from the SDK.
 * @typedef {object} Rules
 * @property {string[]} coreTablePrefixes the prefixes of the tables that are not a plugin's
 * @property {RegExp} idPattern the syntax of a plugin ID
 * @property {string[]} loaderNamePrefixes the starts of the names of variables that load libraries into programs
 * @property {string[]} reservedIds IDs that no plugin may have
 * @property {JsonSchema} schema the manifest's JSON Schema
 * @property {number} sdkMajor the major of the SDK that this build has
 * @property {string[]} secretNameParts the parts of variables' names that mark a secret
 * @property {string[]} sensitiveServices the services that reach sensitive data, such as provider credentials
 * @property {Record<string, number>} services each service of the SDK with its major
 * @property {string} tauriPluginCratePrefix the start of the names of Tauri's plugin crates
 */

/**
 * A manifest that passed the checks.
 * @typedef {object} Manifest
 * @property {string} id
 * @property {number} order
 * @property {{ entry: string }} rust
 * @property {number} sdk
 * @property {{ exclusive?: string[] }} [crates]
 * @property {Partial<Record<import('./native.mjs').NativeCategory, string>>} [native]
 * @property {Record<string, { fixed?: string[], inherit?: string[], reason: string }>} [processes]
 * @property {{ crates?: string[], package: string }[]} [programs]
 * @property {{ entries?: string[], keyed?: string[], legacy?: string[] }} [secrets]
 * @property {Record<string, number>} [services]
 */

/**
 * A plugin that the build bundles.
 * @typedef {object} PluginFolder
 * @property {string} folder its folder, relative to the repository's root, with forward slashes
 * @property {string} id
 * @property {Manifest} manifest
 * @property {boolean} validation whether only validation builds bundle it, as a fixture plugin
 */

/**
 * The folders that hold plugins, relative to the repository's root, and
 * whether only validation builds bundle their plugins.
 */
export const PLUGIN_FOLDERS = [
  { folder: 'plugins', validation: false },
  { folder: 'tests/fixtures/plugins', validation: true },
];

/** The rule names that an ID can break, as `PluginId::parse` checks them. */
const ID_RULES = {
  nestsCorePrefix: 'nests_core_prefix',
  reserved: 'reserved',
  syntax: 'syntax',
  tauriPrefix: 'tauri_prefix',
};

/** The keywords of JSON Schema that `schemaProblems` checks or may ignore. */
const ANNOTATION_KEYWORDS = new Set(['$schema', 'default', 'description', 'format', 'title']);
const CHECKED_KEYWORDS = new Set([
  '$defs',
  '$ref',
  'additionalProperties',
  'items',
  'maximum',
  'minimum',
  'pattern',
  'properties',
  'required',
  'type',
]);

/**
 * The largest values of the integer formats that schemars writes without a
 * `maximum`, as the Rust types that it describes hold them.
 * @type {Record<string, number>}
 */
const FORMAT_MAXIMUMS = { uint16: 65_535, uint32: 4_294_967_295, uint8: 255 };

/**
 * Reads the SDK's rules of manifests.
 * @param {string} root the repository's root
 * @returns {Rules}
 */
export function readRules(root) {
  const sdk = join(root, 'crates', 'mannie-plugin-sdk');
  const rules = JSON.parse(readFileSync(join(sdk, 'plugin-rules.json'), 'utf8'));
  /** @type {JsonSchema} */
  const schema = JSON.parse(readFileSync(join(sdk, 'plugin.schema.json'), 'utf8'));
  const properties = /** @type {Record<string, { pattern: string }>} */ (schema.properties);
  return { ...rules, idPattern: new RegExp(properties.id.pattern, 'u'), schema };
}

/**
 * The prefix of a plugin's tables and message keys: its ID with underscores
 * in place of hyphens.
 * @param {string} id
 */
export function prefixOf(id) {
  return id.replaceAll('-', '_');
}

/**
 * Whether two prefixes nest: whether `<a>_` begins `<b>_` or the other way
 * round, as `prefixes_nest` in the SDK decides.
 * @param {string} a
 * @param {string} b
 */
export function prefixesNest(a, b) {
  const first = `${a}_`;
  const second = `${b}_`;
  return first.startsWith(second) || second.startsWith(first);
}

/**
 * The rule that an ID breaks, in the order of `PluginId::parse`, or `null`
 * for an ID that the host accepts.
 * @param {string} id
 * @param {Rules} rules
 * @returns {{ message: string, rule: string } | null}
 */
export function checkId(id, rules) {
  if (!rules.idPattern.test(id)) {
    return {
      message: `${JSON.stringify(id)} is not a plugin ID, which is lower-case words of letters and digits joined by single hyphens, starting with a letter`,
      rule: ID_RULES.syntax,
    };
  }
  if (rules.reservedIds.includes(id)) {
    return {
      message: `${JSON.stringify(id)} is reserved, because Tauri, the core, the SDK, SQLite or Windows uses the name`,
      rule: ID_RULES.reserved,
    };
  }
  if (id.startsWith(rules.tauriPluginCratePrefix)) {
    return {
      message: `${JSON.stringify(id)} starts with "${rules.tauriPluginCratePrefix}", as the crates of Tauri's plugins do`,
      rule: ID_RULES.tauriPrefix,
    };
  }
  const core = rules.coreTablePrefixes.find((prefix) => prefixesNest(prefixOf(id), prefix));
  if (core !== undefined) {
    return {
      message: `the tables of ${JSON.stringify(id)} would share a prefix with the tables named ${core}_*, which are not a plugin's`,
      rule: ID_RULES.nestsCorePrefix,
    };
  }
  return null;
}

/**
 * The JSON type of a TOML value, as JSON Schema names it.
 * @param {unknown} value
 */
function jsonType(value) {
  if (Array.isArray(value)) {
    return 'array';
  }
  if (Number.isInteger(value)) {
    return 'integer';
  }
  if (value === null) {
    return 'null';
  }
  return typeof value === 'object' ? 'object' : typeof value;
}

/**
 * Where a value sits in the manifest, such as `secrets.entries[0]`.
 * @param {string} path
 * @param {string | number} key
 */
function childPath(path, key) {
  if (typeof key === 'number') {
    return `${path}[${String(key)}]`;
  }
  return path === '' ? key : `${path}.${key}`;
}

/**
 * The places where a value breaks a schema. It checks the keywords that the
 * manifest's schema uses and refuses a schema with any other keyword, so that
 * a later schema cannot pass a manifest that it does not check.
 * @param {unknown} value
 * @param {JsonSchema} schema
 * @param {JsonSchema} root the schema that `$ref` resolves against
 * @param {string} [path]
 * @returns {string[]}
 */
export function schemaProblems(value, schema, root, path = '') {
  for (const keyword of Object.keys(schema)) {
    const isKnown = CHECKED_KEYWORDS.has(keyword) || ANNOTATION_KEYWORDS.has(keyword);
    if (!isKnown) {
      throw new Error(`the manifest's schema uses the keyword ${keyword}, which the generator does not check`);
    }
  }
  if (typeof schema.$ref === 'string') {
    const name = schema.$ref.replace(/^#\/\$defs\//u, '');
    const definitions = /** @type {Record<string, JsonSchema>} */ (root.$defs);
    return schemaProblems(value, definitions[name], root, path);
  }
  const where = path === '' ? 'the manifest' : path;
  const type = jsonType(value);
  // A type is a name or, for a value that may be missing, a list of names.
  const allowed = schema.type === undefined ? [] : [schema.type].flat();
  const isOtherType = allowed.length > 0 && !allowed.includes(type);
  if (isOtherType) {
    return [`${where} must be of type ${allowed.join(' or ')}, not ${type}`];
  }
  /** @type {string[]} */
  const problems = [];
  if (type === 'string') {
    const text = /** @type {string} */ (value);
    const breaksPattern = typeof schema.pattern === 'string' && !new RegExp(schema.pattern, 'u').test(text);
    if (breaksPattern) {
      problems.push(`${where} must match ${String(schema.pattern)}`);
    }
  }
  if (type === 'integer') {
    const number = /** @type {number} */ (value);
    const minimum = typeof schema.minimum === 'number' ? schema.minimum : -Infinity;
    const maximum =
      typeof schema.maximum === 'number' ? schema.maximum : (FORMAT_MAXIMUMS[String(schema.format)] ?? Infinity);
    const isOutOfRange = number < minimum || number > maximum;
    if (isOutOfRange) {
      problems.push(`${where} must be from ${String(minimum)} to ${String(maximum)}`);
    }
  }
  if (type === 'array') {
    const items = /** @type {unknown[]} */ (value);
    const itemSchema = /** @type {JsonSchema | undefined} */ (schema.items);
    if (itemSchema !== undefined) {
      items.forEach((item, index) => {
        problems.push(...schemaProblems(item, itemSchema, root, childPath(path, index)));
      });
    }
  }
  if (type === 'object') {
    const table = /** @type {Record<string, unknown>} */ (value);
    const properties = /** @type {Record<string, JsonSchema>} */ (schema.properties ?? {});
    const required = /** @type {string[]} */ (schema.required ?? []);
    for (const key of required) {
      if (!(key in table)) {
        problems.push(`${where} is missing the field ${key}`);
      }
    }
    for (const [key, field] of Object.entries(table)) {
      const fieldSchema = properties[key] ?? schema.additionalProperties;
      if (fieldSchema === false) {
        problems.push(`${where} has the unknown field ${key}`);
      } else if (typeof fieldSchema === 'object') {
        problems.push(...schemaProblems(field, /** @type {JsonSchema} */ (fieldSchema), root, childPath(path, key)));
      }
    }
  }
  return problems;
}

/**
 * Whether a text is words of ASCII letters, digits, `.`, `_` and `-` joined
 * by single slashes, as the SDK accepts a secret's name.
 * @param {string} name
 */
function isSecretName(name) {
  return name.split('/').every((word) => /^[A-Za-z0-9._-]+$/u.test(word));
}

/**
 * Whether a secret's name is the keyed family `family` or a name below it.
 * @param {string} name
 * @param {string} family
 */
function isInFamily(name, family) {
  const isFamily = name === family;
  const isBelow = name.startsWith(`${family}/`);
  return isFamily || isBelow;
}

/**
 * Whether a text can name an installed program, as `is_process_name` in the
 * SDK decides: a letter or a digit, then letters, digits, `.`, `_` and `-`,
 * at most 64 characters, never ending in a dot.
 * @param {string} name
 */
export function isProcessName(name) {
  const hasSyntax = /^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/u.test(name);
  const endsWithDot = name.endsWith('.');
  return hasSyntax && !endsWithDot;
}

/**
 * Whether an installed program may inherit a variable's name, as the SDK
 * decides: a letter or an underscore, then letters, digits, underscores or
 * parentheses, at most 128 characters, with no part that marks a secret and
 * no start that loads libraries.
 * @param {string} name
 * @param {Rules} rules
 */
export function isInheritable(name, rules) {
  const isName = /^[A-Za-z_][A-Za-z0-9_()]{0,127}$/u.test(name);
  const looksSecret = name.split(/[_()]/u).some((part) => rules.secretNameParts.includes(part.toUpperCase()));
  const loadsLibraries = rules.loaderNamePrefixes.some((prefix) => name.toUpperCase().startsWith(prefix));
  return isName && !looksSecret && !loadsLibraries;
}

/**
 * The first problem of the installed programs under `[processes]`, in the
 * order of `Manifest::parse`, or `null`.
 * @param {Manifest} manifest
 * @param {Rules} rules
 * @returns {string | null}
 */
function processProblem(manifest, rules) {
  const processes = Object.entries(manifest.processes ?? {});
  for (const [program, process] of processes) {
    if (!isProcessName(program)) {
      return `${JSON.stringify(program)} is not the name of an installed program, which is a letter or a digit, then letters, digits, ".", "_" and "-", at most 64 characters, never ending in a dot`;
    }
    // A blank reason, as Rust's `str::trim` sees it, which removes White_Space.
    if (/^\p{White_Space}*$/u.test(process.reason)) {
      return `the manifest runs the program ${program} under [processes] without a reason`;
    }
    if ((process.fixed ?? []).some((argument) => argument.includes('\0'))) {
      return `a fixed argument of the program ${program} holds a NUL character`;
    }
    const refused = (process.inherit ?? []).find((name) => !isInheritable(name, rules));
    if (refused !== undefined) {
      return `the program ${program} inherits ${JSON.stringify(refused)}, which is not a variable's name, looks like a secret's or loads libraries`;
    }
  }
  const listsProcesses = processes.length > 0;
  const declaresProcesses = Object.hasOwn(manifest.services ?? {}, 'processes');
  if (listsProcesses && !declaresProcesses) {
    return 'the manifest lists programs under [processes] but does not declare the service processes';
  }
  return null;
}

/**
 * Parses and checks the text of a `plugin.toml` in the order of
 * `Manifest::parse`: another SDK major first, then the shape, the ID, the
 * secrets' names, the services, the service that secrets need, the reasons
 * of native grants, the installed programs and the service that they need.
 * @param {string} text
 * @param {Rules} rules
 * @returns {{ manifest: Manifest, problems: [] } | { manifest: null, problems: string[] }}
 */
export function checkManifest(text, rules) {
  /** @type {Record<string, unknown>} */
  let table;
  try {
    table = parse(text);
  } catch (error) {
    return { manifest: null, problems: [`the manifest is not valid TOML: ${String(error)}`] };
  }
  // As in the SDK, a major that is not a whole number from 0 to 65535 is left
  // to the check of the shape, which refuses it.
  const sdk = table.sdk;
  const isMajor = typeof sdk === 'number' && Number.isInteger(sdk) && sdk >= 0 && sdk <= 65_535;
  const isOtherMajor = isMajor && sdk !== rules.sdkMajor;
  if (isOtherMajor) {
    return {
      manifest: null,
      problems: [
        `the manifest names major ${String(table.sdk)} of the plugin SDK, and this SDK is major ${String(rules.sdkMajor)}`,
      ],
    };
  }
  // An ID that breaks a rule gets the rule's message, as the host's parse of
  // the ID gives it, in place of the schema's pattern.
  const idProblem = typeof table.id === 'string' ? checkId(table.id, rules) : null;
  if (idProblem !== null) {
    return { manifest: null, problems: [idProblem.message] };
  }
  const shape = schemaProblems(table, rules.schema, rules.schema);
  if (shape.length > 0) {
    return { manifest: null, problems: shape };
  }
  const manifest = /** @type {Manifest} */ (/** @type {unknown} */ (table));
  const entries = manifest.secrets?.entries ?? [];
  const legacy = manifest.secrets?.legacy ?? [];
  const keyed = manifest.secrets?.keyed ?? [];
  const badName = [...entries, ...legacy, ...keyed].find((name) => !isSecretName(name));
  if (badName !== undefined) {
    return {
      manifest: null,
      problems: [
        `${JSON.stringify(badName)} is not a secret's name, which is words of ASCII letters, digits, ".", "_" and "-" joined by single slashes`,
      ],
    };
  }
  const inBoth = entries.find((name) => legacy.includes(name));
  if (inBoth !== undefined) {
    return {
      manifest: null,
      problems: [`the secret ${JSON.stringify(inBoth)} is listed both under entries and under legacy`],
    };
  }
  // A fixed name inside a keyed family would name the same entry as a key of
  // that family (ADR-0036, decision 8).
  const inFamily = [...entries, ...legacy].find((name) => keyed.some((family) => isInFamily(name, family)));
  if (inFamily !== undefined) {
    return {
      manifest: null,
      problems: [`the secret ${JSON.stringify(inFamily)} is a keyed family, or an entry of one`],
    };
  }
  for (const [name, major] of Object.entries(manifest.services ?? {})) {
    const known = rules.services[name];
    if (known === undefined) {
      return {
        manifest: null,
        problems: [`the manifest declares the service ${JSON.stringify(name)}, which the plugin SDK does not have`],
      };
    }
    if (major !== known) {
      return {
        manifest: null,
        problems: [
          `the manifest declares major ${String(major)} of the service ${name}, which the plugin SDK does not have`,
        ],
      };
    }
  }
  const listsSecrets = entries.length > 0 || legacy.length > 0 || keyed.length > 0;
  const declaresSecrets = Object.hasOwn(manifest.services ?? {}, 'secrets');
  if (listsSecrets && !declaresSecrets) {
    return {
      manifest: null,
      problems: ['the manifest lists secrets but does not declare the service secrets'],
    };
  }
  // A blank reason, as Rust's `str::trim` sees it, which removes White_Space.
  const withoutReason = nativeGrants(manifest).find((grant) => /^\p{White_Space}*$/u.test(grant.reason));
  if (withoutReason !== undefined) {
    return {
      manifest: null,
      problems: [`the manifest grants ${withoutReason.category} under [native] without a reason`],
    };
  }
  const process = processProblem(manifest, rules);
  if (process !== null) {
    return { manifest: null, problems: [process] };
  }
  return { manifest, problems: [] };
}

/**
 * The folders that a member of a Cargo workspace names: the path itself, or
 * the folders that a path with `*` or `?` in its last segments matches.
 * @param {string} workspace the workspace's folder
 * @param {string} member a member as `[workspace] members` lists it
 * @returns {string[]} the members' folders, relative to the workspace
 */
export function memberFolders(workspace, member) {
  const segments = member.split('/');
  /** @type {string[]} */
  let folders = [''];
  for (const segment of segments) {
    const isPattern = /[*?]/u.test(segment);
    if (!isPattern) {
      folders = folders.map((folder) => (folder === '' ? segment : `${folder}/${segment}`));
      continue;
    }
    const escaped = segment.replace(/[.+^${}()|[\]\\]/gu, String.raw`\$&`);
    const pattern = new RegExp(`^${escaped.replaceAll('*', '.*').replaceAll('?', '.')}$`, 'u');
    folders = folders.flatMap((folder) => {
      const parent = join(workspace, folder);
      if (!existsSync(parent)) {
        return [];
      }
      return readdirSync(parent, { withFileTypes: true })
        .filter((entry) => entry.isDirectory() && pattern.test(entry.name))
        .map((entry) => (folder === '' ? entry.name : `${folder}/${entry.name}`));
    });
  }
  return folders;
}

/**
 * The problems of a plugin's Cargo workspace that Cargo would report with
 * less context: no `Cargo.toml`, no `[workspace]`, or a member folder without
 * `Cargo.toml`, which Cargo refuses (ADR-0035, context).
 * @param {string} root
 * @param {string} folder the plugin's folder, relative to the root
 * @returns {string[]}
 */
export function workspaceProblems(root, folder) {
  const manifestPath = join(root, folder, 'Cargo.toml');
  if (!existsSync(manifestPath)) {
    return [`${folder} has a plugin.toml but no Cargo.toml, the plugin's Cargo workspace`];
  }
  /** @type {Record<string, unknown>} */
  let cargo;
  try {
    cargo = parse(readFileSync(manifestPath, 'utf8'));
  } catch (error) {
    return [`${folder}/Cargo.toml is not valid TOML: ${String(error)}`];
  }
  const workspace = /** @type {{ members?: string[] } | undefined} */ (cargo.workspace);
  if (workspace === undefined) {
    return [`${folder}/Cargo.toml has no [workspace]: a plugin is a Cargo workspace of its own`];
  }
  /** @type {string[]} */
  const problems = [];
  for (const member of workspace.members ?? []) {
    for (const memberFolder of memberFolders(join(root, folder), member)) {
      if (!existsSync(join(root, folder, memberFolder, 'Cargo.toml'))) {
        problems.push(
          `${folder}/${memberFolder} is a member of the plugin's workspace but has no Cargo.toml; add one or remove the folder`,
        );
      }
    }
  }
  return problems;
}

/** Folders that the walks of a plugin's folder skip. */
const SKIPPED_FOLDERS = new Set(['generated', 'node_modules', 'target']);

/** The names of Clippy's configuration files. */
const CLIPPY_CONFIGURATIONS = new Set(['.clippy.toml', 'clippy.toml']);

/**
 * The Clippy configurations in a plugin's folder. A plugin's crates are
 * linted with the core's configuration (ADR-0035, decision 6), which a file
 * of the plugin's own would replace for the crates below it.
 * @param {string} root
 * @param {string} folder relative to the root
 * @returns {string[]} the files, relative to the root
 */
export function clippyConfigurations(root, folder) {
  /** @type {string[]} */
  const found = [];
  for (const entry of readdirSync(join(root, folder), { withFileTypes: true })) {
    const path = `${folder}/${entry.name}`;
    const isSkipped = SKIPPED_FOLDERS.has(entry.name) || entry.name.startsWith('.');
    if (entry.isFile() && CLIPPY_CONFIGURATIONS.has(entry.name)) {
      found.push(path);
    } else if (entry.isDirectory() && !isSkipped) {
      found.push(...clippyConfigurations(root, path));
    }
  }
  return found;
}

/**
 * Finds every plugin that the build bundles, checks its manifest and its
 * workspace, and refuses two plugins whose prefixes nest. The plugins come
 * in the order of their starts: by `order`, then by ID.
 * @param {string} root the repository's root
 * @param {Rules} rules
 * @returns {{ plugins: PluginFolder[], problems: string[] }}
 */
export function findPlugins(root, rules) {
  /** @type {PluginFolder[]} */
  const plugins = [];
  /** @type {string[]} */
  const problems = [];
  for (const { folder: parent, validation } of PLUGIN_FOLDERS) {
    if (!existsSync(join(root, parent))) {
      continue;
    }
    const folders = readdirSync(join(root, parent), { withFileTypes: true })
      .filter((entry) => entry.isDirectory())
      .map((entry) => entry.name)
      .sort();
    for (const name of folders) {
      const folder = `${parent}/${name}`;
      const manifestPath = join(root, folder, 'plugin.toml');
      if (!existsSync(manifestPath)) {
        problems.push(`${folder} has no plugin.toml; add the plugin's manifest or remove the folder`);
        continue;
      }
      const checked = checkManifest(readFileSync(manifestPath, 'utf8'), rules);
      if (checked.manifest === null) {
        problems.push(...checked.problems.map((problem) => `${folder}/plugin.toml: ${problem}`));
        continue;
      }
      if (checked.manifest.id !== name) {
        problems.push(
          `${folder}/plugin.toml: the ID ${JSON.stringify(checked.manifest.id)} must equal the folder's name, ${JSON.stringify(name)}`,
        );
        continue;
      }
      const folderProblems = [
        ...workspaceProblems(root, folder),
        ...clippyConfigurations(root, folder).map(
          (file) => `${file}: a plugin's crates are linted with the core's Clippy configuration; remove the file`,
        ),
      ];
      problems.push(...folderProblems);
      if (folderProblems.length === 0) {
        plugins.push({ folder, id: name, manifest: checked.manifest, validation });
      }
    }
  }
  plugins.sort((a, b) => a.manifest.order - b.manifest.order || compareText(a.id, b.id));
  for (const [index, plugin] of plugins.entries()) {
    for (const other of plugins.slice(index + 1)) {
      if (prefixesNest(prefixOf(plugin.id), prefixOf(other.id))) {
        problems.push(
          `the plugins ${plugin.id} and ${other.id} have prefixes that nest, so their tables and message keys could not be told apart`,
        );
      }
    }
  }
  return { plugins, problems };
}

/**
 * Orders texts by their code points, as Rust orders strings.
 * @param {string} a
 * @param {string} b
 */
export function compareText(a, b) {
  if (a < b) {
    return -1;
  }
  return a > b ? 1 : 0;
}
