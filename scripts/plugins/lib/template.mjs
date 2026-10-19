// @ts-check
// The fixture plugin as the template of a new plugin (ADR-0035, decision
// 13): its files with every name of the fixture changed to the new plugin's,
// its paths into the SDK moved to the depth of `plugins/<id>/`, and the
// sentences that describe the fixture's role replaced by the new plugin's
// name.

import { prefixOf } from './manifests.mjs';

/** The fixture plugin, the template's source, relative to the root. */
export const TEMPLATE_FOLDER = 'tests/fixtures/plugins/example';
/** The fixture plugin's ID, which the template's names derive from. */
export const TEMPLATE_ID = 'example';

/**
 * The names of a plugin ID in the forms that code uses.
 * @param {string} id such as `log-viewer`
 */
export function namesOf(id) {
  const prefix = prefixOf(id);
  const pascal = id
    .split('-')
    .map((word) => `${word.charAt(0).toUpperCase()}${word.slice(1)}`)
    .join('');
  return { id, pascal, prefix, upper: prefix.toUpperCase() };
}

/**
 * A text of the template with the fixture's names changed to the new
 * plugin's: the crate's names, the table and code prefixes, the type names,
 * the constant of the ID and the ID itself, and the paths into the SDK.
 * @param {string} text
 * @param {string} id the new plugin's ID
 */
export function renamed(text, id) {
  const names = namesOf(id);
  return text
    .replaceAll('PREFIX: &str = "example"', `PREFIX: &str = "${names.prefix}"`)
    .replaceAll('../../../../crates/', '../../crates/')
    .replaceAll('../../../../CHANGELOG.md', '../../CHANGELOG.md')
    .replaceAll('mannie_example', `mannie_${names.prefix}`)
    .replaceAll('example_', `${names.prefix}_`)
    .replaceAll('EXAMPLE', names.upper)
    .replaceAll('Example', names.pascal)
    .replaceAll('example', names.id);
}

/**
 * A path of the template, relative to its folder, in the new plugin's
 * folder.
 * @param {string} path such as `crates/mannie-example/src/lib.rs`
 * @param {string} id
 */
export function renamedPath(path, id) {
  return path.replaceAll('mannie-example', `mannie-${id}`);
}

/**
 * The leading comment of a file, whose lines start with `marker`, replaced
 * by one line. A `#:schema` line before it stays.
 * @param {string} text
 * @param {string} marker such as `#` or `//!`
 * @param {string} line the new comment, without its marker
 */
export function withLeadingComment(text, marker, line) {
  const lines = text.split('\n');
  const start = lines[0].startsWith('#:schema') ? 1 : 0;
  const commentLength = lines.slice(start).findIndex((candidate) => !candidate.startsWith(marker));
  const end = commentLength === -1 ? lines.length : start + commentLength;
  if (end === start) {
    return text;
  }
  return [...lines.slice(0, start), `${marker} ${line}`, ...lines.slice(end)].join('\n');
}

/**
 * The template's file for the new plugin, with the fixture's role replaced
 * by the plugin's name where a file states it: the manifest's and the
 * workspace's leading comments, the crate's description and the crate's
 * documentation, the leading comment of the UI's entry and the plugin's
 * description in its catalogs.
 * @param {string} path relative to the template's folder, with forward slashes
 * @param {string} text the template's text
 * @param {string} id
 */
export function templateFile(path, text, id) {
  const name = `The ${id} plugin.`;
  const isRustCrateRoot = path.endsWith('/src/lib.rs');
  const isPackageManifest = path.startsWith('crates/') && path.endsWith('/Cargo.toml');
  const isManifest = path === 'plugin.toml';
  const isWorkspace = path === 'Cargo.toml';
  const isUiEntry = path === 'ui/index.ts';
  const isCatalog = path.startsWith('messages/') && path.endsWith('.json');
  let result = text;
  if (isUiEntry) {
    result = withLeadingComment(result, '//', `The ${id} plugin's UI.`);
  }
  if (isCatalog) {
    result = result.replace(/^( *"example_plugin_description": ).*?(,?)$/mu, `$1${JSON.stringify(name)}$2`);
  }
  if (isManifest) {
    result = withLeadingComment(result, '#', name);
  }
  if (isWorkspace) {
    result = withLeadingComment(result, '#', `The ${id} plugin's Cargo workspace.`);
  }
  if (isRustCrateRoot) {
    result = withLeadingComment(result, '//!', name);
  }
  if (isPackageManifest) {
    result = result.replace(/^description = ".*"$/mu, `description = ${JSON.stringify(name)}`);
  }
  if (isManifest) {
    result = result.replace(
      /^# After the plugins that ship.*\norder = \d+$/mu,
      '# Where the plugin starts and where the Plugins page lists it.\norder = 50',
    );
  }
  return renamed(result, id);
}
