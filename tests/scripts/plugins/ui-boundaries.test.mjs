// @ts-check
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { describe, it } from 'node:test';

import { importRefusal, uiBoundariesPlugin } from '../../../scripts/plugins/lib/ui-boundaries.mjs';

const repository = join(import.meta.dirname, '..', '..', '..');
const app = join(repository, 'apps', 'desktop');
const workers = join(repository, 'plugins', 'workers');
const example = join(repository, 'tests', 'fixtures', 'plugins', 'example');
const boundaries = { app, plugins: [workers, join(repository, 'plugins', 'log-viewer'), example] };

/** A file of the Workers plugin's UI. */
const workersUi = join(workers, 'ui', 'state', 'workers.svelte.ts');
/** A file of the core's UI. */
const coreUi = join(app, 'src', 'lib', 'state', 'chat-actions.svelte.ts');

describe('importRefusal', () => {
  it("allows a plugin's UI the SDK through its aliases, Svelte and its own files", () => {
    const allowed = [
      join(app, 'src', 'lib', 'sdk', 'ipc.ts'),
      join(app, 'src', 'lib', 'components', 'ui', 'button', 'index.ts'),
      join(app, 'node_modules', '@lucide', 'svelte', 'dist', 'icons', 'server.js'),
      'svelte',
      'svelte/internal/client',
      '../generated/bindings.js',
      './workers-actions.svelte.js',
      `${join(workers, 'ui', 'components', 'WorkerRow.svelte')}`,
      '\0virtual:module',
      '/@vite/client',
    ];

    for (const source of allowed) {
      assert.equal(importRefusal(boundaries, source, workersUi), null, source);
    }
  });

  it("refuses a plugin's UI the core's files, packages and other plugins", () => {
    const refused = [
      '#lib/state/plugin-states.svelte.js',
      '@tauri-apps/api/core',
      'bits-ui',
      '../../../../apps/desktop/src/lib/sdk/ipc.js',
      '../../../log-viewer/ui/index.js',
      join(app, 'src', 'lib', 'state', 'plugin-states.svelte.ts'),
    ];

    for (const source of refused) {
      assert.match(String(importRefusal(boundaries, source, workersUi)), /imports only the SDK/, source);
    }
  });

  it('refuses the transport in every file but the generated ones', () => {
    const transport = join(app, 'src', 'lib', 'sdk', 'transport.ts');

    assert.match(String(importRefusal(boundaries, transport, workersUi)), /only a plugin's generated bindings/);
    assert.match(
      String(importRefusal(boundaries, '#lib/sdk/transport.js', coreUi)),
      /only a plugin's generated bindings/,
    );
    assert.equal(importRefusal(boundaries, transport, join(workers, 'ui', 'generated', 'bindings.ts')), null);
    // The validation harness calls a command that no plugin declares (V1).
    assert.equal(importRefusal(boundaries, transport, join(app, 'src', 'lib', 'validation', 'harness.ts')), null);
  });

  it("refuses the core's UI a plugin's files, which only the generated plugin lists import", () => {
    const pluginFile = '../../../../../plugins/workers/ui/state/workers.svelte.js';

    assert.match(String(importRefusal(boundaries, pluginFile, coreUi)), /only through the generated plugin list/);
    assert.equal(
      importRefusal(
        boundaries,
        '../../../../../plugins/workers/ui/index.js',
        join(app, 'src', 'lib', 'generated', 'plugin-list.ts'),
      ),
      null,
    );
    assert.equal(importRefusal(boundaries, '#lib/state/plugin-states.svelte.js', coreUi), null);
  });

  it("leaves tests and generated files to the lint, and the plugins' barrels reach the compiled catalog", () => {
    const test = join(workers, 'tests', 'ui', 'state', 'workers.test.ts');
    const barrel = join(workers, 'ui', 'generated', 'message-list.ts');

    assert.equal(importRefusal(boundaries, 'vitest', test), null);
    assert.equal(importRefusal(boundaries, '../../../../apps/desktop/src/lib/paraglide/messages.js', barrel), null);
  });
});

describe('uiBoundariesPlugin', () => {
  it('throws the refusal and resolves nothing itself', () => {
    const plugin = uiBoundariesPlugin(boundaries);

    assert.equal(plugin.resolveId('svelte', workersUi), null);
    assert.equal(plugin.resolveId('./entry.ts', undefined), null);
    assert.throws(() => plugin.resolveId('@tauri-apps/api/core', `${workersUi}?v=1`), /imports only the SDK/);
  });
});
