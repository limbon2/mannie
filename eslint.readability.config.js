// @ts-check
import { defineConfig } from 'eslint/config';

import base from './eslint.config.js';
import { declarationPadding, statementGroupPadding } from './scripts/eslint/statement-group-spacing.mjs';

// The audit also checks layouts that Prettier can print differently and Svelte
// changes that need review for behavior and rendered whitespace.
export default defineConfig(
  base,
  {
    name: 'mannie/readability-audit',
    files: ['**/*.{js,mjs,cjs,ts,mts,cts,tsx,svelte}'],
    rules: {
      '@stylistic/brace-style': 'error',
      '@stylistic/object-curly-newline': ['error', { ObjectExpression: { minProperties: 4, multiline: true } }],
      '@stylistic/array-element-newline': ['error', 'consistent'],
      '@stylistic/function-call-argument-newline': ['error', 'consistent'],
      '@stylistic/padding-line-between-statements': ['error', ...declarationPadding, ...statementGroupPadding],
      'mannie/multiline-object-properties': ['error', { wrapCallbacks: true }],
      'mannie/named-type-annotations': 'error',
    },
  },
  {
    name: 'mannie/readability-audit-script-indent',
    files: ['**/*.{js,mjs,cjs,ts,mts,cts,tsx}'],
    rules: { '@stylistic/indent': ['error', 2] },
  },
  {
    name: 'mannie/readability-audit-svelte',
    files: ['**/*.svelte'],
    rules: {
      'svelte/indent': ['error', { indent: 2 }],
      'svelte/max-attributes-per-line': ['error', { singleline: 3, multiline: 1 }],
      'svelte/first-attribute-linebreak': 'error',
      'svelte/html-closing-bracket-new-line': ['error', { singleline: 'never', multiline: 'always' }],
      'mannie/no-complex-inline-handler': 'error',
      'mannie/svelte-script-padding': 'error',
      'mannie/template-newlines': 'error',
      'mannie/template-section-spacing': 'error',
    },
  },
);
