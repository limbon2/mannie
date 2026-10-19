// @ts-check
import { join } from 'node:path';
import { describe, it } from 'node:test';

import { RuleTester } from 'eslint';

import { testFileLocation } from '../../../scripts/eslint/test-file-location.mjs';

RuleTester.describe = describe;
RuleTester.it = it;
RuleTester.itOnly = it.only;

const repository = join(import.meta.dirname, '..', '..', '..');
const options = [{ pluginsFolder: join(repository, 'plugins'), testsFolder: join(repository, 'tests') }];

/**
 * @param {string} path Repository-relative path of the linted file.
 */
function file(path) {
  return { code: 'export {};', filename: join(repository, path), options };
}

new RuleTester().run('test-file-location', testFileLocation, {
  valid: [
    file('tests/scripts/eslint/named-conditions.test.mjs'),
    file('tests/desktop/lib/format.test.ts'),
    file('scripts/eslint/named-conditions.mjs'),
    file('apps/desktop/src/lib/format.ts'),
    // A file whose name only contains "test" is not a test file.
    file('apps/desktop/src/lib/latest.ts'),
    // A plugin's tests live in its folder's tests/.
    file('plugins/workers/tests/ui/state/workers.test.ts'),
    file('plugins/workers/tests/e2e/workers.test.ts'),
    file('tests/fixtures/plugins/example/tests/ui/index.test.ts'),
  ],
  invalid: [
    { ...file('scripts/eslint/named-conditions.test.mjs'), errors: [{ messageId: 'move' }] },
    { ...file('apps/desktop/src/lib/format.spec.ts'), errors: [{ messageId: 'move' }] },
    { ...file('apps/desktop/src/routes/page.svelte.test.ts'), errors: [{ messageId: 'move' }] },
    // A folder named like tests/ elsewhere does not count.
    { ...file('apps/desktop/tests/format.test.ts'), errors: [{ messageId: 'move' }] },
    // Nor a plugin's test beside its code, or a tests/ folder below the plugin's own.
    { ...file('plugins/workers/ui/state/workers.test.ts'), errors: [{ messageId: 'move' }] },
    { ...file('plugins/workers/ui/tests/workers.test.ts'), errors: [{ messageId: 'move' }] },
    { ...file('plugins/tests/workers.test.ts'), errors: [{ messageId: 'move' }] },
  ],
});
