// @ts-check
import { join } from 'node:path';
import { describe, it } from 'node:test';

import { RuleTester } from 'eslint';

import { pluginBoundaries } from '../../../scripts/eslint/plugin-boundaries.mjs';

RuleTester.describe = describe;
RuleTester.it = it;
RuleTester.itOnly = it.only;

const repository = join(import.meta.dirname, '..', '..', '..');
const testsFolder = join(repository, 'tests');
const options = [
  {
    plugins: [
      { fixture: false, folder: join(repository, 'plugins', 'workers'), id: 'workers', prefix: 'workers' },
      { fixture: false, folder: join(repository, 'plugins', 'log-viewer'), id: 'log-viewer', prefix: 'log_viewer' },
      {
        fixture: true,
        folder: join(repository, 'tests', 'fixtures', 'plugins', 'example'),
        id: 'example',
        prefix: 'example',
      },
    ],
    testsFolder,
  },
];

/**
 * @param {string} path Path of the linted file, relative to the repository.
 * @param {string} code
 */
function file(path, code) {
  return { code, filename: join(repository, path), options };
}

new RuleTester().run('plugin-boundaries', pluginBoundaries, {
  valid: [
    // A plugin's UI imports the SDK, Svelte and its own files, its generated ones included.
    file('plugins/workers/ui/index.ts', "import { definePlugin } from '@mannie/plugin-sdk';"),
    file('plugins/workers/ui/state/workers.svelte.ts', "import { call } from '@mannie/plugin-sdk/ipc';"),
    file(
      'plugins/workers/ui/components/WorkerRow.svelte',
      "import { Button } from '@mannie/plugin-sdk/components/button';",
    ),
    file('plugins/workers/ui/components/WorkerRow.svelte', "import ServerIcon from '@mannie/plugin-sdk/icons/server';"),
    file('plugins/workers/ui/components/WorkersPanel.svelte', "import { onMount } from 'svelte';"),
    file('plugins/workers/ui/state/workers.svelte.ts', "import { SvelteMap } from 'svelte/reactivity';"),
    file('plugins/workers/ui/index.ts', "import WorkersPanel from './components/WorkersPanel.svelte';"),
    file('plugins/workers/ui/utils/workers-messages.ts', "import { m } from '../generated/messages.js';"),
    file('plugins/workers/ui/state/workers.svelte.ts', "export { channels } from '../generated/bindings.js';"),
    // A plugin's tests import packages, the shared end-to-end support and the plugin's own files.
    file('plugins/workers/tests/ui/state/workers.test.ts', "import { describe } from 'vitest';"),
    file(
      'plugins/workers/tests/ui/state/workers.test.ts',
      "import { WorkerList } from '../../../ui/state/workers.svelte.js';",
    ),
    file('plugins/workers/tests/e2e/workers.test.ts', "import { waitForApp } from '#e2e/page.ts';"),
    file('plugins/workers/tests/e2e/workers.test.ts', "import { join } from 'node:path';"),
    // The core imports the core, the SDK and the generated plugin list, and names its own messages.
    file('apps/desktop/src/lib/registrations/compose-shell.ts', "import { plugins } from '#plugin-list';"),
    file('apps/desktop/src/lib/state/chat-actions.svelte.ts', "import { commands } from '#lib/generated/bindings.js';"),
    file('apps/desktop/src/lib/utils/chat-errors.ts', 'const text = m.chat_error_plugin_off({ name });'),
    file('apps/desktop/src/lib/utils/chat-errors.ts', 'const text = messages.workers_pair();'),
    // The core's tests use the fixture plugins.
    file(
      'tests/apps/desktop/src/lib/registrations/compose-shell.test.ts',
      "import example from '../../../../../fixtures/plugins/example/ui/index.js';",
    ),
    // A plugin names its own message keys.
    file('plugins/workers/ui/utils/workers-messages.ts', 'const text = m.workers_pair();'),
  ],
  invalid: [
    {
      // The core never imports a plugin's UI.
      ...file(
        'apps/desktop/src/lib/components/app/chat/TaskCard.svelte',
        "import { workerList } from '../../../../../../../plugins/workers/ui/state/workers.svelte.js';",
      ),
      errors: [{ data: { plugin: 'workers' }, messageId: 'core' }],
    },
    {
      // Nor does it call a plugin's commands (T-35).
      ...file(
        'apps/desktop/src/lib/state/chat-actions.svelte.ts',
        "export { commands } from '../../../../../plugins/workers/ui/generated/bindings.js';",
      ),
      errors: [{ data: { plugin: 'workers' }, messageId: 'core' }],
    },
    {
      // The app's code never uses a fixture plugin, which only its tests do.
      ...file(
        'apps/desktop/src/lib/state/chat-actions.svelte.ts',
        "const example = import('../../../../../tests/fixtures/plugins/example/ui/index.js');",
      ),
      errors: [{ data: { plugin: 'example' }, messageId: 'core' }],
    },
    {
      // The core's tests never use a real plugin.
      ...file(
        'tests/apps/desktop/src/lib/registrations/compose-shell.test.ts',
        "import workers from '../../../../../../plugins/workers/ui/index.js';",
      ),
      errors: [{ data: { plugin: 'workers' }, messageId: 'core' }],
    },
    {
      // Nor does the core name a plugin's messages.
      ...file('apps/desktop/src/lib/utils/plugins-messages.ts', 'const name = m.workers_plugin_name();'),
      errors: [{ data: { key: 'workers_plugin_name', plugin: 'workers' }, messageId: 'messageKey' }],
    },
    {
      ...file(
        'tests/apps/desktop/src/lib/utils/plugins-messages.test.ts',
        'expect(m.log_viewer_plugin_name()).toBe("");',
      ),
      errors: [{ data: { key: 'log_viewer_plugin_name', plugin: 'log-viewer' }, messageId: 'messageKey' }],
    },
    {
      // A plugin's UI never imports the core's files, Tauri, SvelteKit or a package.
      ...file(
        'plugins/workers/ui/state/workers.svelte.ts',
        "import { pluginStates } from '#lib/state/plugin-states.svelte.js';",
      ),
      errors: [{ data: { source: '#lib/state/plugin-states.svelte.js' }, messageId: 'pluginUiImport' }],
    },
    {
      ...file('plugins/workers/ui/state/workers.svelte.ts', "import { invoke } from '@tauri-apps/api/core';"),
      errors: [{ messageId: 'pluginUiImport' }],
    },
    {
      ...file('plugins/workers/ui/state/workers-actions.svelte.ts', "import { goto } from '$app/navigation';"),
      errors: [{ messageId: 'pluginUiImport' }],
    },
    {
      ...file(
        'plugins/workers/ui/components/WorkerRow.svelte',
        "import ServerIcon from '@lucide/svelte/icons/server';",
      ),
      errors: [{ messageId: 'pluginUiImport' }],
    },
    {
      ...file(
        'plugins/workers/ui/components/WorkerRow.svelte',
        "import { Button } from '../../../../apps/desktop/src/lib/components/ui/button/index.js';",
      ),
      errors: [{ messageId: 'pluginUiImport' }],
    },
    {
      // Nor another plugin's files.
      ...file('plugins/log-viewer/ui/index.ts', "import { commands } from '../../workers/ui/generated/bindings.js';"),
      errors: [{ data: { plugin: 'workers' }, messageId: 'otherPlugin' }],
    },
    {
      // A plugin's tests never import the core's files.
      ...file(
        'plugins/workers/tests/ui/state/workers.test.ts',
        "import { pluginStates } from '../../../../../apps/desktop/src/lib/state/plugin-states.svelte.js';",
      ),
      errors: [{ messageId: 'pluginTestImport' }],
    },
    {
      ...file('plugins/workers/tests/ui/state/workers.test.ts', "import { m } from '#lib/paraglide/messages.js';"),
      errors: [{ messageId: 'pluginTestImport' }],
    },
  ],
});
