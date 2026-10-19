// @ts-check
// mannie/test-file-location: test files live under tests/, never beside code.
//
// tests/ mirrors the folders of the code it tests: the tests of
// scripts/eslint/ are in tests/scripts/eslint/. A plugin keeps its tests in
// its folder's tests/, which mirrors the plugin's folder: the tests of
// plugins/workers/ui/ are in plugins/workers/tests/ui/ (ADR-0035,
// decision 9). A file is a test file when its name contains `.test.` or
// `.spec.`, the names Vitest and Node's test runner look for. The
// `testsFolder` option is the absolute path of tests/, and `pluginsFolder`
// the absolute path of plugins/.
import { basename, isAbsolute, relative, sep } from 'node:path';

const TEST_FILE_NAME = /\.(?:spec|test)\./;

/**
 * Whether `path` is inside `folder`.
 * @param {string} folder
 * @param {string} path
 */
function isWithin(folder, path) {
  const within = relative(folder, path);
  const leaves = within === '..' || within.startsWith(`..${sep}`);
  return !leaves && !isAbsolute(within);
}

/** @type {import('eslint').Rule.RuleModule} */
export const testFileLocation = {
  meta: {
    type: 'suggestion',
    docs: {
      description: 'Require test files to live under the tests folder',
    },
    messages: {
      move: "Move this test file under tests/, or a plugin's under its folder's tests/, in the folder that mirrors the code it tests.",
    },
    schema: [
      {
        additionalProperties: false,
        properties: { pluginsFolder: { type: 'string' }, testsFolder: { type: 'string' } },
        required: ['pluginsFolder', 'testsFolder'],
        type: 'object',
      },
    ],
  },
  create(context) {
    const [{ pluginsFolder, testsFolder }] = /** @type {[{ pluginsFolder: string; testsFolder: string }]} */ (
      context.options
    );
    const inPlugins = isWithin(pluginsFolder, context.filename)
      ? relative(pluginsFolder, context.filename).split(sep)
      : [];
    const isInPluginTests = inPlugins.length > 2 && inPlugins[1] === 'tests';
    const isOutsideTests = !isWithin(testsFolder, context.filename) && !isInPluginTests;
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
