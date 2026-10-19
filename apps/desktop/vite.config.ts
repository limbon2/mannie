/// <reference types="vitest/config" />
// SvelteKit 3 takes its configuration in the Vite plugin (ADR-0005): a static
// single-page app built with adapter-static, with an index.html fallback and
// no server-side rendering, because Tauri serves it from its own protocol.
//
// The `validation` mode builds the app for the end-to-end validations: it
// compiles the pseudo-locale catalog that `task gen:pseudo-locale` generates
// (ADR-0019), loads the validation harness (src/routes/+layout.ts) and lists
// the fixture plugins too (ADR-0035, decision 7).
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { paraglideVitePlugin } from '@inlang/paraglide-js';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { defaultClientConditions, defineConfig } from 'vite';

import { uiBoundariesPlugin } from '../../scripts/plugins/lib/ui-boundaries.mjs';

/** A path relative to the app's folder, absolute and with forward slashes, as aliases name it. */
function fromApp(path: string): string {
  return fileURLToPath(new URL(path, import.meta.url)).replaceAll('\\', '/');
}

/** The folders of the plugins that `task gen` found, the fixture plugins' included, from its index. */
function pluginFolders(): string[] {
  const index = fromApp('../../.generated/desktop/plugins.json');
  if (!existsSync(index)) {
    throw new Error('The index of the plugins, .generated/desktop/plugins.json, does not exist. Run `task gen`.');
  }
  const plugins = JSON.parse(readFileSync(index, 'utf8')) as readonly { readonly folder: string }[];
  return plugins.map((plugin) => fromApp(`../../${plugin.folder}`));
}

export default defineConfig(({ mode }) => {
  const validation = mode === 'validation';
  // Both inlang projects are generated: `task gen` writes the app's, with the
  // core's catalogs and each plugin's, and `task gen:pseudo-locale` the
  // validation builds' (ADR-0035, decision 8).
  const project = validation ? './.pseudo-locale/project.inlang' : './.generated/project.inlang';
  if (!existsSync(fileURLToPath(new URL(`${project}/settings.json`, import.meta.url)))) {
    const task = validation ? 'task gen:pseudo-locale' : 'task gen';
    throw new Error(`The inlang project ${project} does not exist. Run \`${task}\`.`);
  }
  return {
    clearScreen: false,
    plugins: [
      // Refuses imports across the boundaries of the plugins' UI (ADR-0035,
      // decision 6), after the SDK's aliases below.
      uiBoundariesPlugin({ app: fromApp('.'), plugins: pluginFolders() }),
      tailwindcss(),
      sveltekit({
        adapter: adapter({ fallback: 'index.html' }),
        compilerOptions: {
          // Runes mode for every project file; dependencies keep their own mode.
          runes: ({ filename }) => (filename.split(/[/\\]/).includes('node_modules') ? undefined : true),
        },
      }),
      paraglideVitePlugin({ project }),
    ],
    resolve: {
      alias: [
        // The TypeScript plugin SDK, which plugins' UI and generated bindings
        // import by name (ADR-0034, decisions 9 and 11): the installed
        // shadcn-svelte components and the icons by their names, then the
        // SDK's own modules. A plugin folder has no package.json, so these
        // are absolute paths. tsconfig.json maps the same names for the
        // type checks.
        {
          find: /^@mannie\/plugin-sdk\/components\/([a-z-]+)$/,
          replacement: `${fromApp('src/lib/components/ui')}/$1/index.ts`,
        },
        {
          find: /^@mannie\/plugin-sdk\/icons\/([a-z0-9-]+)$/,
          replacement: `${fromApp('node_modules/@lucide/svelte/dist/icons')}/$1.js`,
        },
        { find: /^@mannie\/plugin-sdk\/([a-z-]+)$/, replacement: `${fromApp('src/lib/sdk')}/$1.ts` },
        { find: /^@mannie\/plugin-sdk$/, replacement: fromApp('src/lib/sdk/index.ts') },
        // The browser build of this micromark dependency decodes character
        // references by assigning innerHTML; its default build looks them up
        // in a table, so the Markdown path never creates markup (ADR-0021,
        // decision 7).
        {
          find: 'decode-named-character-reference',
          replacement: fromApp('node_modules/decode-named-character-reference/index.js'),
        },
      ],
      // Validation builds resolve `#plugin-list` to the list with the
      // fixture plugins (package.json; ADR-0035, decision 7).
      ...(validation ? { conditions: ['validation', ...defaultClientConditions] } : {}),
    },
    // Tauri expects a fixed port, and the dev server only listens locally.
    server: { host: '127.0.0.1', port: 5173, strictPort: true },
    test: {
      // The core's tests live in the repository's tests/ folder, which
      // mirrors the code, and each plugin's in its folder's tests/ui
      // (ADR-0035, decision 9).
      dir: '../..',
      environment: 'node',
      include: [
        'tests/apps/desktop/**/*.test.ts',
        'plugins/*/tests/ui/**/*.test.ts',
        'tests/fixtures/plugins/*/tests/ui/**/*.test.ts',
      ],
    },
  };
});
