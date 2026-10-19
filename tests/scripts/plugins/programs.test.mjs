// @ts-check
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import {
  archiveName,
  buildCommands,
  dependencyClosure,
  FORBIDDEN_CRATE,
  freebsdReleaseScript,
  freebsdScript,
  matrices,
  unknownTargets,
} from '../../../scripts/plugins/lib/programs.mjs';

/** @type {import('../../../scripts/plugins/lib/programs.mjs').Program[]} */
const PROGRAMS = [
  {
    e2e: true,
    folder: 'plugins/workers',
    headless: true,
    package: 'mannie-worker',
    plugin: 'workers',
    selfCheck: ['self-check', '--json'],
    signed: true,
    smoke: { helpers: ['--example', 'smoke-master'], unix: 'ci/smoke.sh', windows: 'ci/smoke.ps1' },
    targets: ['x86_64-pc-windows-msvc', 'x86_64-unknown-freebsd', 'x86_64-unknown-linux-musl'],
  },
  {
    e2e: false,
    folder: 'plugins/notes',
    headless: false,
    package: 'mannie-notes-tool',
    plugin: 'notes',
    selfCheck: [],
    signed: false,
    smoke: { helpers: [], unix: null, windows: null },
    targets: ['x86_64-pc-windows-msvc'],
  },
];

/**
 * A resolve graph from edges of the form [from, to, kind].
 * @param {[string, string, string | null][]} edges
 */
function metadata(edges) {
  const names = [...new Set(edges.flatMap(([from, to]) => [from, to]))];
  return {
    packages: names.map((name) => ({ id: `${name}-id`, name })),
    resolve: {
      nodes: names.map((name) => ({
        deps: edges
          .filter(([from]) => from === name)
          .map(([, to, kind]) => ({ dep_kinds: [{ kind }], pkg: `${to}-id` })),
        id: `${name}-id`,
      })),
    },
  };
}

describe('dependencyClosure', () => {
  it('follows normal and build dependencies, not development dependencies', () => {
    const graph = metadata([
      ['mannie-worker', 'mannie-store', null],
      ['mannie-store', 'rusqlite', null],
      ['rusqlite', 'cc', 'build'],
      ['mannie-worker', 'tempfile', 'dev'],
      ['tempfile', 'fastrand', null],
    ]);

    assert.deepEqual([...dependencyClosure(graph, 'mannie-worker')].sort(), ['cc', 'mannie-store', 'rusqlite']);
  });

  it('survives cycles', () => {
    const graph = metadata([
      ['mannie-worker', 'a', null],
      ['a', 'b', null],
      ['b', 'a', null],
    ]);

    assert.deepEqual([...dependencyClosure(graph, 'mannie-worker')].sort(), ['a', 'b']);
  });
});

describe('FORBIDDEN_CRATE', () => {
  it('matches the desktop stack', () => {
    for (const name of [
      'tauri',
      'tauri-runtime-wry',
      'wry',
      'tao',
      'gtk',
      'gtk-sys',
      'gdk-pixbuf',
      'glib',
      'webkit2gtk',
      'webkit2gtk-sys',
      'soup3',
    ]) {
      assert.match(name, FORBIDDEN_CRATE);
    }
  });

  it('leaves similar names alone', () => {
    for (const name of ['taoism', 'glibc-version', 'gtkish', 'tokio', 'windows-service']) {
      assert.doesNotMatch(name, FORBIDDEN_CRATE);
    }
  });
});

describe('buildCommands', () => {
  it('builds every program, or those of the end-to-end tests, from their workspaces', () => {
    assert.deepEqual(buildCommands(PROGRAMS, { e2e: false, release: true }), [
      ['build', '--locked', '--release', '--manifest-path', 'plugins/workers/Cargo.toml', '-p', 'mannie-worker'],
      ['build', '--locked', '--release', '--manifest-path', 'plugins/notes/Cargo.toml', '-p', 'mannie-notes-tool'],
    ]);
    assert.deepEqual(buildCommands(PROGRAMS, { e2e: true, release: false }), [
      ['build', '--locked', '--manifest-path', 'plugins/workers/Cargo.toml', '-p', 'mannie-worker'],
    ]);
  });
});

describe('matrices', () => {
  it('gives a smoke test for each script and a Linux build for each target but Windows', () => {
    const { builds, musl, smoke, windows } = matrices(PROGRAMS);

    assert.deepEqual(smoke, [
      {
        folder: 'plugins/workers',
        helpers: '--example smoke-master',
        os: 'ubuntu-24.04',
        package: 'mannie-worker',
        script: 'plugins/workers/ci/smoke.sh',
      },
      {
        folder: 'plugins/workers',
        helpers: '--example smoke-master',
        os: 'windows-11',
        package: 'mannie-worker',
        script: 'plugins/workers/ci/smoke.ps1',
      },
    ]);
    assert.deepEqual(builds, [
      { folder: 'plugins/workers', package: 'mannie-worker', target: 'x86_64-unknown-freebsd', tool: 'cross' },
      { folder: 'plugins/workers', package: 'mannie-worker', target: 'x86_64-unknown-linux-musl', tool: 'cargo' },
    ]);
    assert.deepEqual(musl, [builds[1]]);
    assert.deepEqual(windows, [
      { folder: 'plugins/workers', package: 'mannie-worker' },
      { folder: 'plugins/notes', package: 'mannie-notes-tool' },
    ]);
  });
});

describe('freebsdScript', () => {
  it('tests and smoke-tests the programs with a FreeBSD target with cargo alone', () => {
    assert.equal(
      freebsdScript(PROGRAMS),
      [
        'set -eu',
        'cargo test --locked --manifest-path plugins/workers/Cargo.toml --workspace --all-features',
        'cargo build --locked --manifest-path plugins/workers/Cargo.toml -p mannie-worker',
        'cargo build --locked --manifest-path plugins/workers/Cargo.toml --example smoke-master',
        'sh plugins/workers/ci/smoke.sh target/debug',
        '',
      ].join('\n'),
    );
  });
});

describe('archiveName', () => {
  it('names an archive by package, version and platform', () => {
    assert.equal(
      archiveName('mannie-worker', '0.3.0', 'x86_64-pc-windows-msvc'),
      'mannie-worker-0.3.0-windows-x86_64.zip',
    );
    assert.equal(
      archiveName('mannie-worker', '0.3.0', 'x86_64-unknown-linux-musl'),
      'mannie-worker-0.3.0-linux-x86_64.tar.gz',
    );
    assert.throws(() => archiveName('mannie-worker', '0.3.0', 'aarch64-apple-darwin'), /no archive/u);
  });

  it('refuses a target that the release builds no archive for', () => {
    const planted = [{ ...PROGRAMS[1], targets: ['aarch64-apple-darwin'] }];

    assert.deepEqual(unknownTargets(PROGRAMS), []);
    assert.deepEqual(unknownTargets(planted), [
      'plugins/notes/plugin.toml: the release builds no archive for the target aarch64-apple-darwin of mannie-notes-tool',
    ]);
  });
});

describe('freebsdReleaseScript', () => {
  it('builds and packs each program with a FreeBSD target with cargo and tar', () => {
    assert.equal(
      freebsdReleaseScript(PROGRAMS),
      [
        'set -eu',
        'mkdir -p dist',
        'cargo build --locked --release --manifest-path plugins/workers/Cargo.toml -p mannie-worker',
        'mkdir -p staging/mannie-worker',
        'cp target/release/mannie-worker LICENSE staging/mannie-worker/',
        'tar -czf "dist/mannie-worker-$VERSION-freebsd-x86_64.tar.gz" -C staging/mannie-worker .',
        '',
      ].join('\n'),
    );
  });
});
