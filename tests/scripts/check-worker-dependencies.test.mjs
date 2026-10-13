// @ts-check
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { dependencyClosure, FORBIDDEN_CRATE } from '../../scripts/check-worker-dependencies.mjs';

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
