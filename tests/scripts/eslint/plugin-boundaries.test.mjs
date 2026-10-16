// @ts-check
import { join } from 'node:path';
import { describe, it } from 'node:test';

import { RuleTester } from 'eslint';

import { pluginBoundaries } from '../../../scripts/eslint/plugin-boundaries.mjs';

RuleTester.describe = describe;
RuleTester.it = it;
RuleTester.itOnly = it.only;

const repository = join(import.meta.dirname, '..', '..', '..');
const libFolder = join(repository, 'apps', 'desktop', 'src', 'lib');
const options = [{ libFolder, plugins: ['workers', 'jira'] }];

/**
 * @param {string} path Path of the linted file, relative to the app's src/lib/.
 * @param {string} code
 */
function file(path, code) {
  return { code, filename: join(libFolder, path), options };
}

new RuleTester().run('plugin-boundaries', pluginBoundaries, {
  valid: [
    // The composition root's list imports each plugin's registration.
    file('registrations/plugin-list.ts', "import { workersPlugin } from './workers.js';"),
    file('registrations/plugin-list.ts', "import { jiraPlugin } from '#lib/registrations/jira.js';"),
    // A plugin's UI imports its own files and the core's.
    file('registrations/workers.ts', "import WorkersPanel from '#lib/components/app/workers/WorkersPanel.svelte';"),
    file('registrations/workers.ts', "import { workerList } from '../state/workers.svelte.js';"),
    file('components/app/workers/WorkerRow.svelte', "import { m } from '#lib/paraglide/messages.js';"),
    file('state/workers-actions.svelte.ts', "import { pluginStates } from './plugin-states.svelte.js';"),
    file('utils/workers-messages.ts', "import { pluginName } from '#lib/state/plugin-catalog.js';"),
    // The core imports the core, packages and files whose names only start like a plugin's ID.
    file('components/app/chat/TaskCard.svelte', "import { Button } from '#lib/components/ui/button/index.js';"),
    file('utils/markdown.ts', "import Worker from './markdown-worker.ts?worker';"),
    file('state/chat-actions.svelte.ts', "import { commands } from '#lib/generated/bindings.js';"),
    file('state/chat-actions.svelte.ts', "import { onMount } from 'svelte';"),
    file('registrations/compose-shell.ts', "import { plugins } from './plugin-list.js';"),
    file('state/plugin-states.svelte.ts', "export { batchChannel } from '#lib/utils/calls.js';"),
  ],
  invalid: [
    {
      ...file('components/app/chat/TaskCard.svelte', "import { workerList } from '#lib/state/workers.svelte.js';"),
      errors: [{ data: { plugin: 'workers' }, messageId: 'core' }],
    },
    {
      ...file('registrations/features.ts', "import { workersPlugin } from './workers.js';"),
      errors: [{ messageId: 'core' }],
    },
    {
      ...file('utils/chat-errors.ts', "export { workersRefusalMessage } from './workers-messages.js';"),
      errors: [{ messageId: 'core' }],
    },
    {
      ...file(
        'state/conversation.svelte.ts',
        "const panel = import('#lib/components/app/workers/WorkersPanel.svelte');",
      ),
      errors: [{ messageId: 'core' }],
    },
    // The list of plugins imports registrations only.
    {
      ...file('registrations/plugin-list.ts', "import { workerList } from '../state/workers.svelte.js';"),
      errors: [{ messageId: 'core' }],
    },
    // A plugin never imports another plugin.
    {
      ...file('registrations/jira.ts', "import { workerList } from '#lib/state/workers.svelte.js';"),
      errors: [{ data: { plugin: 'workers' }, messageId: 'otherPlugin' }],
    },
    {
      ...file('components/app/jira/JiraPanel.svelte', "import WorkerRow from '../workers/WorkerRow.svelte';"),
      errors: [{ messageId: 'otherPlugin' }],
    },
  ],
});
