// @ts-check
// mannie/test-file-location: test files live under tests/, never beside code.
//
// tests/ mirrors the folders of the code it tests: the tests of
// scripts/eslint/ are in tests/scripts/eslint/. A file is a test file when its
// name contains `.test.` or `.spec.`, the names Vitest and Node's test runner
// look for. The `testsFolder` option is the absolute path of tests/.
import { basename, isAbsolute, relative, sep } from 'node:path';

const TEST_FILE_NAME = /\.(?:spec|test)\./;

/** @type {import('eslint').Rule.RuleModule} */
export const testFileLocation = {
  meta: {
    type: 'suggestion',
    docs: {
      description: 'Require test files to live under the tests folder',
    },
    messages: {
      move: 'Move this test file under tests/, in the folder that mirrors the code it tests.',
    },
    schema: [
      {
        additionalProperties: false,
        properties: { testsFolder: { type: 'string' } },
        required: ['testsFolder'],
        type: 'object',
      },
    ],
  },
  create(context) {
    const [{ testsFolder }] = /** @type {[{ testsFolder: string }]} */ (context.options);
    const pathInTests = relative(testsFolder, context.filename);
    const leavesTests = pathInTests === '..' || pathInTests.startsWith(`..${sep}`);
    const isOutsideTests = leavesTests || isAbsolute(pathInTests);
    const isTestFile = TEST_FILE_NAME.test(basename(context.filename));
    return {
      Program(node) {
        if (isTestFile && isOutsideTests) {
          context.report({ messageId: 'move', node });
        }
      },
    };
  },
};
