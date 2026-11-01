// @ts-check
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { firstBundle, XTERM_MARKER, xtermInFirstBundle } from '../../scripts/check-first-bundle.mjs';

/** A manifest with an entry, a route, a chunk that both share and a chunk that only a dynamic import reaches. */
const MANIFEST = {
  '.svelte-kit/generated/build/client-optimized/app.js': {
    dynamicImports: ['src/lib/sdk/terminal.ts'],
    file: '_app/immutable/entry/app.js',
    imports: ['_shared.js'],
    isEntry: true,
  },
  '.svelte-kit/generated/build/client-optimized/nodes/0.js': {
    file: '_app/immutable/nodes/0.js',
    imports: ['_shared.js'],
    isEntry: true,
  },
  _shared: { file: '_app/immutable/chunks/unused.js' },
  '_shared.js': { file: '_app/immutable/chunks/shared.js' },
  'node_modules/@xterm/xterm/lib/xterm.mjs': { file: '_app/immutable/chunks/xterm.js', isDynamicEntry: true },
  'src/lib/sdk/terminal.ts': {
    dynamicImports: ['node_modules/@xterm/xterm/lib/xterm.mjs'],
    file: '_app/immutable/chunks/terminal.js',
    isDynamicEntry: true,
  },
};

describe('firstBundle', () => {
  it('holds the entries and what they import statically, each once, and no chunk that only a dynamic import reaches', () => {
    assert.deepEqual(firstBundle(MANIFEST).sort(), [
      '_app/immutable/chunks/shared.js',
      '_app/immutable/entry/app.js',
      '_app/immutable/nodes/0.js',
    ]);
  });
});

describe('xtermInFirstBundle', () => {
  it('finds nothing while xterm.js loads only through a dynamic import', () => {
    const read = (/** @type {string} */ file) => (file.endsWith('xterm.js') ? XTERM_MARKER : 'app code');

    assert.deepEqual(xtermInFirstBundle(MANIFEST, read), []);
  });

  it('names a chunk of the first bundle that holds xterm.js', () => {
    const read = (/** @type {string} */ file) => (file.endsWith('shared.js') ? `class ${XTERM_MARKER}` : 'app code');

    assert.deepEqual(xtermInFirstBundle(MANIFEST, read), ['_app/immutable/chunks/shared.js']);
  });
});
