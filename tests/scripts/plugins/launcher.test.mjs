// @ts-check
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { parse } from 'smol-toml';

import {
  editorWorkspace,
  launcherManifest,
  pluginBindingsCrate,
  pluginListCrate,
  tomlValue,
} from '../../../scripts/plugins/lib/launcher.mjs';

/**
 * A TOML text as plain JSON values; the parser makes its tables without a
 * prototype, which strict equality tells apart from object literals.
 * @param {string} text
 * @returns {any}
 */
function toml(text) {
  return JSON.parse(JSON.stringify(parse(text)));
}

/** @type {import('../../../scripts/plugins/lib/launcher.mjs').LauncherPlugin[]} */
const PLUGINS = [
  {
    crateFolder: 'plugins/workers/crates/mannie-workers',
    entry: 'mannie-workers',
    folder: 'plugins/workers',
    id: 'workers',
    validation: false,
  },
  {
    crateFolder: 'tests/fixtures/plugins/example/crates/mannie-example',
    entry: 'mannie-example',
    folder: 'tests/fixtures/plugins/example',
    id: 'example',
    validation: true,
  },
];

describe('tomlValue', () => {
  it('writes values on one line, with inline tables', () => {
    assert.equal(
      tomlValue({ 'default-features': false, features: ['std'], version: '0.3.34' }),
      '{ default-features = false, features = ["std"], version = "0.3.34" }',
    );
    assert.equal(tomlValue({ level: 'warn', priority: -1 }), '{ level = "warn", priority = -1 }');
    assert.equal(tomlValue('a "quoted" name'), String.raw`"a \"quoted\" name"`);
  });
});

describe('launcherManifest', () => {
  const rootManifest = parse(`
[workspace]
resolver = "3"
members = ["crates/*"]

[workspace.package]
version = "0.2.0"
edition = "2024"

[workspace.dependencies]
anyhow = "1.0.104"
mannie-types = { path = "crates/mannie-types" }

[workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }

[profile.release]
lto = true
`);
  const text = launcherManifest({
    binaryFolder: 'apps/desktop/src-tauri',
    coreFolder: 'apps/desktop/core',
    launcherFolder: '.generated/desktop',
    plugins: PLUGINS,
    rootManifest,
  });
  const launcher = toml(text);

  it("takes the root's settings and rebases the paths of its dependencies", () => {
    const workspace = launcher.workspace;

    assert.deepEqual(workspace.members, ['../../apps/desktop/src-tauri', 'plugin-bindings', 'plugin-list']);
    assert.deepEqual(workspace.package, { edition: '2024', version: '0.2.0' });
    assert.equal(workspace.dependencies.anyhow, '1.0.104');
    assert.deepEqual(workspace.dependencies['mannie-types'], { path: '../../crates/mannie-types' });
    assert.deepEqual(workspace.lints.clippy.pedantic, { level: 'warn', priority: -1 });
    assert.deepEqual(launcher.profile.release, { lto: true });
  });

  it('adds the desktop app, the plugin list and every plugin entry crate', () => {
    const dependencies = launcher.workspace.dependencies;

    assert.deepEqual(dependencies['mannie-desktop-core'], { path: '../../apps/desktop/core' });
    assert.deepEqual(dependencies['mannie-plugin-list'], { path: 'plugin-list' });
    assert.deepEqual(dependencies['mannie-workers'], { path: '../../plugins/workers/crates/mannie-workers' });
    assert.deepEqual(dependencies['mannie-example'], {
      path: '../../tests/fixtures/plugins/example/crates/mannie-example',
    });
  });

  it('lists the dependencies sorted by name', () => {
    const names = Object.keys(launcher.workspace.dependencies);

    assert.deepEqual(names, [...names].sort());
  });
});

describe('pluginListCrate', () => {
  it('bundles the fixture plugins only behind the feature validation', () => {
    const crate = pluginListCrate(PLUGINS);
    const manifest = toml(crate.cargoToml);

    assert.deepEqual(manifest.dependencies['mannie-example'], { optional: true, workspace: true });
    assert.deepEqual(manifest.features.validation, ['dep:mannie-example']);
    assert.match(
      crate.libRs,
      /#\[cfg\(not\(feature = "validation"\)\)\]\npub fn plugins\(\) -> Vec<PluginEntry> \{\n {4}vec!\[\n {8}mannie_workers::mannie_plugin\(\),\n {4}\]\n\}/u,
    );
    assert.match(crate.libRs, /mannie_workers::mannie_plugin\(\),\n {8}mannie_example::mannie_plugin\(\),/u);
  });

  it('writes one list when no fixture plugin exists', () => {
    const crate = pluginListCrate(PLUGINS.slice(0, 1));

    assert.doesNotMatch(crate.libRs, /cfg/u);
    assert.deepEqual(toml(crate.cargoToml).features.validation, []);
  });
});

describe('pluginBindingsCrate', () => {
  it("writes every plugin's bindings, the fixture plugins' too, with their folders", () => {
    const crate = pluginBindingsCrate(PLUGINS);

    assert.match(
      crate.mainRs,
      /folder: "plugins\/workers",\n {12}validation: false,\n {12}entry: mannie_workers::mannie_plugin\(\),/u,
    );
    assert.match(crate.mainRs, /folder: "tests\/fixtures\/plugins\/example",\n {12}validation: true,/u);
    assert.deepEqual(Object.keys(toml(crate.cargoToml).dependencies), [
      'mannie-example',
      'mannie-plugin-sdk',
      'mannie-workers',
      'serde_json',
    ]);
  });
});

describe('editorWorkspace', () => {
  it('links the workspaces in the order given, with absolute paths', () => {
    const workspace = JSON.parse(editorWorkspace('D:\\work\\mannie', ['Cargo.toml', '.generated/desktop/Cargo.toml']));

    assert.deepEqual(workspace.settings['rust-analyzer.linkedProjects'], [
      'D:/work/mannie/Cargo.toml',
      'D:/work/mannie/.generated/desktop/Cargo.toml',
    ]);
  });
});
