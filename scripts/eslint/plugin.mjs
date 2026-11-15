// @ts-check
// Mannie's own ESLint rules. eslint.config.js enables them; each rule file
// explains what it checks, and tests/scripts/eslint/ shows the accepted and
// rejected code (`task test:scripts`).
import { multilineObjectProperties } from './multiline-object-properties.mjs';
import { namedConditions } from './named-conditions.mjs';
import { namedTypeAnnotations } from './named-type-annotations.mjs';
import { noComplexInlineHandler } from './no-complex-inline-handler.mjs';
import { pluginBoundaries } from './plugin-boundaries.mjs';
import { svelteScriptOrder } from './svelte-script-order.mjs';
import { svelteScriptPadding } from './svelte-script-padding.mjs';
import { templateNewlines } from './template-newlines.mjs';
import { templateSectionSpacing } from './template-section-spacing.mjs';
import { testFileLocation } from './test-file-location.mjs';

/** @type {import('eslint').ESLint.Plugin} */
export default {
  meta: { name: 'mannie' },
  rules: {
    'multiline-object-properties': multilineObjectProperties,
    'named-conditions': namedConditions,
    'named-type-annotations': namedTypeAnnotations,
    'no-complex-inline-handler': noComplexInlineHandler,
    'plugin-boundaries': pluginBoundaries,
    'svelte-script-order': svelteScriptOrder,
    'svelte-script-padding': svelteScriptPadding,
    'template-newlines': templateNewlines,
    'template-section-spacing': templateSectionSpacing,
    'test-file-location': testFileLocation,
  },
};
