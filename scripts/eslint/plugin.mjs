// @ts-check
// Mannie's own ESLint rules. eslint.config.js enables them; each rule file
// explains what it checks, and tests/scripts/eslint/ shows the accepted and
// rejected code (`task test:scripts`).
import { namedConditions } from './named-conditions.mjs';
import { pluginBoundaries } from './plugin-boundaries.mjs';
import { svelteScriptOrder } from './svelte-script-order.mjs';
import { testFileLocation } from './test-file-location.mjs';

/** @type {import('eslint').ESLint.Plugin} */
export default {
  meta: { name: 'mannie' },
  rules: {
    'named-conditions': namedConditions,
    'plugin-boundaries': pluginBoundaries,
    'svelte-script-order': svelteScriptOrder,
    'test-file-location': testFileLocation,
  },
};
