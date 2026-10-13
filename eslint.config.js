// @ts-check
// ESLint for every TypeScript, Svelte and JavaScript file. `task lint:ts` runs
// it with --max-warnings 0, so a warning fails like an error. `task fmt` applies
// the fixable rules, including the sorting rules, before Prettier formats.
// Prettier owns the layout, the Tailwind class order (prettier-plugin-tailwindcss)
// and CSS declaration order; ESLint owns correctness and the order of imports,
// exports, type members and Svelte attributes.
import { join } from 'node:path';

import comments from '@eslint-community/eslint-plugin-eslint-comments/configs';
import { includeIgnoreFile } from '@eslint/compat';
import js from '@eslint/js';
import { loadConfig } from '@sveltejs/load-config';
import prettier from 'eslint-config-prettier';
import tailwind from 'eslint-plugin-better-tailwindcss';
import perfectionist from 'eslint-plugin-perfectionist';
import svelte from 'eslint-plugin-svelte';
import { defineConfig, globalIgnores } from 'eslint/config';
import globals from 'globals';
import tseslint from 'typescript-eslint';

import mannie from './scripts/eslint/plugin.mjs';

const desktopApp = join(import.meta.dirname, 'apps/desktop');

// SvelteKit 3 keeps the Svelte options in vite.config. Loading them gives the
// Svelte rules the forced runes mode and the compiler's warning filter. Before
// the app exists there is nothing to load; a config that fails to load stops
// the lint instead of silently linting without it.
const loadedSvelteConfig = await loadConfig(desktopApp, { traverse: false });
const svelteConfigFailed = loadedSvelteConfig !== undefined && 'error' in loadedSvelteConfig;
if (svelteConfigFailed) {
  throw loadedSvelteConfig.error;
}
const svelteConfig = loadedSvelteConfig?.config;

// Tailwind's built-in palette. Mannie's components use the theme tokens instead.
const tailwindPalette = [
  'amber',
  'black',
  'blue',
  'cyan',
  'emerald',
  'fuchsia',
  'gray',
  'green',
  'indigo',
  'lime',
  'mauve',
  'mist',
  'neutral',
  'olive',
  'orange',
  'pink',
  'purple',
  'red',
  'rose',
  'sky',
  'slate',
  'stone',
  'taupe',
  'teal',
  'violet',
  'white',
  'yellow',
  'zinc',
];

// Attributes whose literal values people read or hear.
const textAttributes = [
  'alt',
  'aria-description',
  'aria-label',
  'aria-placeholder',
  'aria-roledescription',
  'aria-valuetext',
  'label',
  'placeholder',
  'title',
];

const typeScriptFiles = ['**/*.ts', '**/*.mts', '**/*.cts'];
const svelteFiles = ['**/*.svelte', '**/*.svelte.ts'];
const javaScriptFiles = ['**/*.js', '**/*.mjs', '**/*.cjs'];

/**
 * Attribute order of Svelte markup: identity, styling, the other attributes and
 * props, bindings, event handlers, then directives. Svelte 5 event handlers are
 * plain attributes (`onclick`, `onOpenChange`), so they are matched by the "on"
 * prefix. Actions keep their written order because they run in that order.
 * @type {unknown[]}
 */
const svelteAttributeOrder = [
  'this',
  'bind:this',
  'id',
  'name',
  'slot',
  { match: '/^--/u', sort: 'alphabetical' },
  'class',
  { match: '/^class:/u', sort: 'alphabetical' },
  'style',
  { match: '/^style:/u', sort: 'alphabetical' },
  {
    match: ['!/:/u', '!/^(?:this|id|name|slot|class|style)$/u', '!/^--/u', '!/^on/u'],
    sort: 'alphabetical',
  },
  { match: ['/^bind:/u', '!bind:this'], sort: 'alphabetical' },
  { match: '/^on/u', sort: 'alphabetical' },
  { match: '/^use:/u', sort: 'ignore' },
  { match: '/^transition:/u', sort: 'alphabetical' },
  { match: '/^in:/u', sort: 'alphabetical' },
  { match: '/^out:/u', sort: 'alphabetical' },
  { match: '/^animate:/u', sort: 'alphabetical' },
  { match: '/^let:/u', sort: 'alphabetical' },
];

export default defineConfig(
  includeIgnoreFile(join(import.meta.dirname, '.gitignore'), 'Paths ignored by Git'),
  globalIgnores(
    // shadcn-svelte components stay CLI output (ADR-0006): Prettier formats
    // them, but rules that would demand edits do not apply.
    ['apps/desktop/src/lib/components/ui/**'],
    'shadcn-svelte components',
  ),

  {
    name: 'mannie/linter-options',
    linterOptions: {
      // A suppression that no longer suppresses anything is an error.
      reportUnusedDisableDirectives: 'error',
      reportUnusedInlineConfigs: 'error',
    },
  },

  js.configs.recommended,
  tseslint.configs.strictTypeChecked,
  tseslint.configs.stylisticTypeChecked,
  svelte.configs.recommended,
  svelte.configs.prettier,
  comments.recommended,

  {
    name: 'mannie/parser',
    languageOptions: {
      parserOptions: {
        // One set of options for TypeScript and Svelte files, because the
        // TypeScript parser keeps the options of its first initialization.
        extraFileExtensions: ['.svelte'],
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
  },
  {
    name: 'mannie/svelte-parser',
    files: svelteFiles,
    languageOptions: {
      parserOptions: { parser: tseslint.parser, svelteConfig },
    },
  },
  {
    name: 'mannie/javascript',
    files: javaScriptFiles,
    // Scripts and tool configuration are JavaScript outside every tsconfig, so
    // they get the rules that need no type information.
    extends: [tseslint.configs.disableTypeChecked],
    languageOptions: { globals: globals.node },
  },

  {
    name: 'mannie/rules',
    plugins: { mannie, perfectionist },
    settings: {
      perfectionist: { ignoreCase: true, type: 'natural' },
    },
    rules: {
      // Suppressions are narrow and say why: `// eslint-disable-next-line rule -- reason`.
      '@eslint-community/eslint-comments/no-use': [
        'error',
        { allow: ['eslint-disable', 'eslint-enable', 'eslint-disable-line', 'eslint-disable-next-line'] },
      ],
      '@eslint-community/eslint-comments/require-description': 'error',

      // A compound condition combines named parts (scripts/eslint/named-conditions.mjs).
      'mannie/named-conditions': 'error',
      // Tests live under tests/, never beside the code they test.
      'mannie/test-file-location': ['error', { testsFolder: join(import.meta.dirname, 'tests') }],

      eqeqeq: ['error', 'always', { null: 'ignore' }],
      'no-alert': 'error',
      'no-console': 'error',
      'no-constructor-return': 'error',
      'no-duplicate-imports': ['error', { allowSeparateTypeImports: true }],
      'no-else-return': ['error', { allowElseIf: false }],
      'no-lonely-if': 'error',
      'no-nested-ternary': 'error',
      'no-param-reassign': 'error',
      'no-promise-executor-return': 'error',
      'no-self-compare': 'error',
      'no-template-curly-in-string': 'error',
      'no-unmodified-loop-condition': 'error',
      'no-unneeded-ternary': ['error', { defaultAssignment: false }],
      'no-unreachable-loop': 'error',
      'no-useless-rename': 'error',
      'object-shorthand': ['error', 'always'],
      'prefer-template': 'error',

      '@typescript-eslint/no-unused-vars': [
        'error',
        {
          args: 'all',
          argsIgnorePattern: '^_',
          caughtErrors: 'all',
          caughtErrorsIgnorePattern: '^_',
          destructuredArrayIgnorePattern: '^_',
          ignoreRestSiblings: true,
          varsIgnorePattern: '^_',
        },
      ],

      // Sorting. Imports are grouped by origin; a type import sorts with the
      // value imports of the same origin and goes first when the paths are equal.
      'perfectionist/sort-exports': 'error',
      'perfectionist/sort-heritage-clauses': 'error',
      'perfectionist/sort-imports': [
        'error',
        {
          customGroups: [
            { elementNamePattern: String.raw`^\$(?:app|env|service-worker)(?:/|$)`, groupName: 'sveltekit' },
          ],
          fallbackSort: { type: 'type-import-first' },
          groups: [
            'side-effect-style',
            'side-effect',
            ['type-builtin', 'value-builtin'],
            ['type-external', 'value-external'],
            'sveltekit',
            ['type-internal', 'value-internal'],
            ['type-parent', 'value-parent'],
            ['type-sibling', 'value-sibling', 'type-index', 'value-index'],
            'unknown',
          ],
          // SvelteKit 3 replaces the $lib alias with the #lib subpath import.
          internalPattern: [String.raw`^#lib(?:/|$)`],
          newlinesBetween: 1,
        },
      ],
      'perfectionist/sort-interfaces': ['error', { partitionByNewLine: true }],
      'perfectionist/sort-named-exports': 'error',
      'perfectionist/sort-named-imports': 'error',
      'perfectionist/sort-object-types': ['error', { partitionByNewLine: true }],
      'perfectionist/sort-union-types': ['error', { groups: ['unknown', 'nullish'] }],
    },
  },
  {
    name: 'mannie/typescript',
    files: [...typeScriptFiles, ...svelteFiles],
    rules: {
      '@typescript-eslint/consistent-type-exports': 'error',
      '@typescript-eslint/consistent-type-imports': [
        'error',
        { fixStyle: 'separate-type-imports', prefer: 'type-imports' },
      ],
      '@typescript-eslint/explicit-module-boundary-types': 'error',
      // `onclick={() => save()}` stays a one-liner.
      '@typescript-eslint/no-confusing-void-expression': ['error', { ignoreArrowShorthand: true }],
      '@typescript-eslint/no-import-type-side-effects': 'error',
      '@typescript-eslint/prefer-readonly': 'error',
      '@typescript-eslint/require-array-sort-compare': ['error', { ignoreStringArrays: true }],
      // As strict-type-checked, except that numbers may appear in template literals.
      '@typescript-eslint/restrict-template-expressions': [
        'error',
        {
          allowAny: false,
          allowBoolean: false,
          allowNever: false,
          allowNullish: false,
          allowNumber: true,
          allowRegExp: false,
        },
      ],
      '@typescript-eslint/switch-exhaustiveness-check': 'error',
    },
  },
  {
    name: 'mannie/svelte',
    files: svelteFiles,
    rules: {
      // TypeScript already checks what these core rules check, as in *.ts files.
      ...tseslint.configs.eslintRecommended.rules,
      // Svelte's version understands $props and $derived.
      'prefer-const': 'off',
      'svelte/prefer-const': 'error',

      'svelte/block-lang': ['error', { script: 'ts', style: null }],
      'svelte/button-has-type': 'error',
      'svelte/comment-directive': ['error', { reportUnusedDisableDirectives: true }],
      'svelte/no-add-event-listener': 'error',
      'svelte/no-at-const-tags': 'error',
      'svelte/no-bind-value-on-checkable-inputs': 'error',
      'svelte/no-conflicting-module-names': 'error',
      'svelte/no-ignored-unsubscribe': 'error',
      'svelte/no-nested-style-tag': 'error',
      'svelte/no-target-blank': 'error',
      'svelte/prefer-derived-over-derived-by': 'error',
      // Style directives set properties through the CSSOM, which a strict CSP allows.
      'svelte/prefer-style-directive': 'error',
      'svelte/sort-attributes': ['error', { order: svelteAttributeOrder }],
      // Imports, types, props, state, derived values, functions, effects
      // (scripts/eslint/svelte-script-order.mjs).
      'mannie/svelte-script-order': 'error',
      // Compiler warnings, including the accessibility checks, fail the lint.
      'svelte/valid-compile': 'error',
      'svelte/valid-style-parse': 'error',
    },
  },
  {
    name: 'mannie/desktop-app',
    files: ['apps/desktop/src/**/*.svelte', 'apps/desktop/src/**/*.ts'],
    plugins: { 'better-tailwindcss': tailwind },
    settings: {
      'better-tailwindcss': { entryPoint: join(desktopApp, 'src/app.css') },
    },
    rules: {
      // The webview reaches the core only through the generated IPC bindings,
      // and state lives in runes (engineering standards, TypeScript and Svelte).
      'no-restricted-globals': [
        'error',
        ...['EventSource', 'WebSocket', 'XMLHttpRequest', 'fetch'].map((name) => ({
          message: 'The webview has no network access: call the core through the generated IPC bindings.',
          name,
        })),
      ],
      'no-restricted-imports': [
        'error',
        {
          paths: [
            { message: 'Use $app/state.', name: '$app/stores' },
            {
              importNames: ['invoke'],
              message: 'Call commands through the generated IPC bindings.',
              name: '@tauri-apps/api/core',
            },
            {
              importNames: ['derived', 'readable', 'writable'],
              message: 'Keep state in runes ($state, $derived) in .svelte.ts modules.',
              name: 'svelte/store',
            },
          ],
        },
      ],
      // Visible text comes from the Paraglide message catalog (ADR-0019).
      'no-restricted-syntax': [
        'error',
        {
          message: 'Visible text comes from the message catalog (ADR-0019): call a message function.',
          selector: 'SvelteText[value=/[A-Za-z]/]:not(SvelteStyleElement > SvelteText)',
        },
        {
          message: 'Visible text comes from the message catalog (ADR-0019): call a message function.',
          selector: `SvelteAttribute[key.name=/^(?:${textAttributes.join('|')})$/] > SvelteLiteral[value=/[A-Za-z]/]`,
        },
      ],

      ...tailwind.configs.correctness.rules,
      // Covers the shorthand, important-position and variable-syntax rules.
      'better-tailwindcss/enforce-canonical-classes': 'error',
      // Start and end instead of left and right, so layouts can mirror for
      // right-to-left languages. Block-direction and size utilities stay physical.
      'better-tailwindcss/enforce-logical-properties': [
        'error',
        {
          ignore: [
            String.raw`(?:^|:)-?(?:scroll-)?[mp][tb]-`,
            String.raw`(?:^|:)-?(?:top|bottom)-`,
            String.raw`(?:^|:)border-[tb](?:-|$)`,
            String.raw`(?:^|:)(?:min-|max-)?[hw]-`,
            String.raw`(?:^|:)size-`,
          ],
        },
      ],
      'better-tailwindcss/no-deprecated-classes': 'error',
      'better-tailwindcss/no-duplicate-classes': 'error',
      // Colors come from the theme tokens (engineering standards, TypeScript and Svelte).
      'better-tailwindcss/no-restricted-classes': [
        'error',
        {
          restrict: [
            {
              message: 'Hard-coded color: use a theme token such as bg-primary or text-muted-foreground.',
              pattern: `^(?:.*:)?!?-?[a-z]+(?:-[a-z]+)*-(?:${tailwindPalette.join('|')})(?:-\\d{2,3})?(?:/\\S+)?!?$`,
            },
            {
              message: 'Hard-coded color in an arbitrary value: use a theme token.',
              pattern: String.raw`\[[^\]]*(?:#[\da-fA-F]{3,8}|(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color)\()`,
            },
          ],
        },
      ],
      'better-tailwindcss/no-unnecessary-whitespace': 'error',
      // Class order and line wrapping belong to Prettier.
    },
  },
  {
    name: 'mannie/desktop-static',
    // Plain scripts that the webview loads before the app, such as theme.js.
    files: ['apps/desktop/static/**/*.js'],
    languageOptions: { globals: globals.browser },
  },
  {
    name: 'mannie/scripts',
    files: ['scripts/**'],
    rules: {
      // Command-line tools report on the console.
      'no-console': 'off',
    },
  },
  {
    name: 'mannie/e2e',
    files: ['tests/e2e/**'],
    rules: {
      // The end-to-end tests print the measurements that the validation records cite.
      'no-console': 'off',
    },
  },

  // Last: turn off every rule that would fight Prettier's layout.
  prettier,
  {
    name: 'mannie/after-prettier',
    rules: {
      // eslint-config-prettier turns curly off; "all" never conflicts with Prettier.
      curly: ['error', 'all'],
    },
  },
);
