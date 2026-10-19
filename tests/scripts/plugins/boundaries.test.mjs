// @ts-check
import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { after, describe, it } from 'node:test';

import {
  boundaryProblems,
  globReexportProblems,
  shippedLibraryPackages,
} from '../../../scripts/plugins/lib/boundaries.mjs';

const ROOT = '/repo';
const LAUNCHER = '.generated/desktop';

/** @type {import('../../../scripts/plugins/lib/boundaries.mjs').BoundaryPlugin} */
const WORKERS = {
  entry: 'mannie-workers',
  exclusive: ['tonic', 'tonic-prost'],
  folder: 'plugins/workers',
  id: 'workers',
  programs: ['mannie-worker', 'mannie-link-worker'],
};

/** @type {import('../../../scripts/plugins/lib/boundaries.mjs').BoundaryPlugin[]} */
const PLUGINS = [
  WORKERS,
  { entry: 'mannie-images', exclusive: ['image'], folder: 'plugins/images', id: 'images', programs: [] },
];

/**
 * A package of the graph: a path package in a folder of the repository, or
 * a registry crate for a name without a folder.
 * @param {string} name
 * @param {string} [folder]
 */
function cratePackage(name, folder) {
  return {
    dependencies: [],
    id: `${name}-id`,
    manifest_path: folder === undefined ? `/registry/${name}/Cargo.toml` : `${ROOT}/${folder}/Cargo.toml`,
    name,
    source: folder === undefined ? 'registry+https://github.com/rust-lang/crates.io-index' : null,
    targets: [],
    version: '0.2.0',
  };
}

/** Where the path packages of the graphs live. */
const FOLDERS = {
  'mannie-desktop': 'apps/desktop/src-tauri',
  'mannie-desktop-core': 'apps/desktop/core',
  'mannie-host-kit': 'crates/mannie-host-kit',
  'mannie-images': 'plugins/images/crates/mannie-images',
  'mannie-link': 'plugins/workers/crates/mannie-link',
  'mannie-link-worker': 'plugins/workers/crates/mannie-link-worker',
  'mannie-plugin-harness': 'crates/mannie-plugin-harness',
  'mannie-plugin-list': '.generated/desktop/plugin-list',
  'mannie-plugin-sdk': 'crates/mannie-plugin-sdk',
  'mannie-runtime': 'crates/mannie-runtime',
  'mannie-types': 'crates/mannie-types',
  'mannie-worker': 'plugins/workers/worker',
  'mannie-workers': 'plugins/workers/crates/mannie-workers',
  'mannie-workers-testing': 'plugins/workers/crates/mannie-workers-testing',
};

/**
 * A graph of `cargo metadata` from edges `[from, to]`, or `[from, to, 'dev']`
 * for a dev-dependency.
 * @param {([string, string] | [string, string, 'dev'])[]} edges
 */
function graph(edges) {
  const names = [...new Set(edges.flatMap(([from, to]) => [from, to]))];
  return {
    metadata: {
      packages: names.map((name) => cratePackage(name, /** @type {Record<string, string>} */ (FOLDERS)[name])),
      resolve: {
        nodes: names.map((name) => ({
          deps: edges
            .filter(([from]) => from === name)
            .map(([, to, kind]) => ({ dep_kinds: [{ kind: kind ?? null, target: null }], pkg: `${to}-id` })),
          id: `${name}-id`,
        })),
      },
      workspace_members: [],
      workspace_root: ROOT,
    },
    name: 'launcher',
  };
}

/**
 * The problems of edges.
 * @param {([string, string] | [string, string, 'dev'])[]} edges
 */
function problemsOf(edges) {
  return boundaryProblems({
    graphs: [graph(edges)],
    launcherFolder: LAUNCHER,
    plugins: PLUGINS,
    root: ROOT,
  });
}

describe('boundaryProblems', () => {
  it('accepts the edges that the rules allow', () => {
    const problems = problemsOf([
      ['mannie-desktop', 'mannie-desktop-core'],
      ['mannie-desktop', 'mannie-plugin-list'],
      ['mannie-plugin-list', 'mannie-workers'],
      ['mannie-desktop-core', 'mannie-runtime'],
      ['mannie-workers', 'mannie-link'],
      ['mannie-workers', 'mannie-plugin-sdk'],
      ['mannie-link', 'tonic'],
      ['tonic-prost', 'tonic'],
      ['mannie-images', 'image'],
      ['mannie-images', 'serde'],
    ]);

    assert.deepEqual(problems, []);
  });

  it("refuses the core's edge to a plugin's crate", () => {
    assert.deepEqual(problemsOf([['mannie-desktop-core', 'mannie-workers']]), [
      'launcher: mannie-desktop-core -> mannie-workers: the core depends on the plugin workers',
    ]);
  });

  it("refuses a plugin's edge to another plugin", () => {
    assert.deepEqual(problemsOf([['mannie-images', 'mannie-link']]), [
      'launcher: mannie-images -> mannie-link: the plugin images depends on the plugin workers',
    ]);
  });

  it("refuses a plugin's edge to a core crate", () => {
    assert.deepEqual(problemsOf([['mannie-workers', 'mannie-runtime']]), [
      'launcher: mannie-workers -> mannie-runtime: the plugin workers reaches the core only through the plugin SDK (ADR-0034, decision 2)',
    ]);
  });

  it('refuses an exclusive crate outside its plugin and outside its list', () => {
    assert.deepEqual(
      problemsOf([
        ['mannie-desktop-core', 'image'],
        ['axum', 'tonic'],
      ]),
      [
        'launcher: axum -> tonic, but only the plugin workers may use tonic, which its manifest lists under [crates] exclusive',
        'launcher: mannie-desktop-core -> image, but only the plugin images may use image, which its manifest lists under [crates] exclusive',
      ],
    );
  });

  it("lets a plugin's tests use the harness: a dev-dependency, or a crate that only tests reach", () => {
    const problems = problemsOf([
      ['mannie-images', 'mannie-plugin-harness', 'dev'],
      ['mannie-workers', 'mannie-workers-testing', 'dev'],
      ['mannie-workers-testing', 'mannie-workers'],
      ['mannie-workers-testing', 'mannie-plugin-harness'],
      ['mannie-workers-testing', 'mannie-host-kit'],
    ]);

    assert.deepEqual(problems, []);
  });

  it('refuses the harness in what a plugin ships', () => {
    assert.deepEqual(problemsOf([['mannie-images', 'mannie-plugin-harness']]), [
      'launcher: mannie-images -> mannie-plugin-harness: only the tests of the plugin images may use the plugin harness, as a dev-dependency or from a crate that only tests reach (ADR-0034, decision 1)',
      'launcher: the entry crate mannie-images of the plugin images ships the plugin harness, which only its tests may use (ADR-0034, decision 2)',
    ]);
  });

  it("lets a plugin's programs and their crates use the host kit, and refuses it in what the entry crate ships", () => {
    const allowed = problemsOf([
      ['mannie-worker', 'mannie-host-kit'],
      ['mannie-worker', 'mannie-link-worker'],
      ['mannie-link-worker', 'mannie-host-kit'],
      ['mannie-link-worker', 'mannie-link'],
    ]);
    const shipped = problemsOf([
      ['mannie-workers', 'mannie-link'],
      ['mannie-workers', 'mannie-link-worker'],
      ['mannie-link-worker', 'mannie-host-kit'],
      ['mannie-link', 'mannie-host-kit'],
    ]);

    assert.deepEqual(allowed, []);
    assert.deepEqual(shipped, [
      'launcher: mannie-link -> mannie-host-kit: only the programs of the plugin workers, and the crates that its manifest lists under them, may use the host kit (ADR-0034, decision 12)',
      'launcher: the entry crate mannie-workers of the plugin workers ships the host kit, which only its programs may use (ADR-0034, decision 2)',
    ]);
  });

  it('refuses two plugins that list one crate', () => {
    const plugins = [
      ...PLUGINS,
      { entry: 'mannie-other', exclusive: ['image'], folder: 'plugins/other', id: 'other', programs: [] },
    ];

    const problems = boundaryProblems({ graphs: [], launcherFolder: LAUNCHER, plugins, root: ROOT });

    assert.deepEqual(problems, ['the plugins images and other both list image under [crates] exclusive']);
  });
});

describe('globReexportProblems', () => {
  const scratch = mkdtempSync(join(tmpdir(), 'mannie-boundaries-'));
  after(() => {
    rmSync(scratch, { force: true, recursive: true });
  });

  it('refuses a glob re-export in the SDK and accepts named ones', () => {
    mkdirSync(join(scratch, 'crates/mannie-plugin-sdk/src/points'), { recursive: true });
    writeFileSync(
      join(scratch, 'crates/mannie-plugin-sdk/src/lib.rs'),
      'pub use crate::ids::PluginId;\nuse std::fmt::*;\n',
    );
    writeFileSync(
      join(scratch, 'crates/mannie-plugin-sdk/src/points/tools.rs'),
      '//! Tools.\n\npub use mannie_types::tools::*;\n',
    );

    assert.deepEqual(globReexportProblems(scratch), [
      'crates/mannie-plugin-sdk/src/points/tools.rs:3: the plugin SDK re-exports items one by one, never with a glob',
    ]);
  });
});

describe('shippedLibraryPackages', () => {
  it("lists the entry crate and the plugin's crates that it ships, by name", () => {
    const { metadata } = graph([
      ['mannie-workers', 'mannie-link'],
      ['mannie-workers', 'mannie-plugin-sdk'],
      ['mannie-workers', 'tonic'],
      ['mannie-workers', 'mannie-workers-testing', 'dev'],
      ['mannie-workers-testing', 'mannie-link-worker'],
      ['mannie-worker', 'mannie-link-worker'],
      ['mannie-link-worker', 'mannie-link'],
    ]);

    const names = shippedLibraryPackages(metadata, WORKERS, ROOT).map((entry) => entry.name);

    assert.deepEqual(names, ['mannie-link', 'mannie-workers']);
  });

  it('lists nothing for a workspace without the entry crate', () => {
    const { metadata } = graph([['mannie-link', 'tonic']]);

    assert.deepEqual(shippedLibraryPackages(metadata, WORKERS, ROOT), []);
  });
});
