/// <reference types="vitest/config" />
// SvelteKit 3 takes its configuration in the Vite plugin (ADR-0005): a static
// single-page app built with adapter-static, with an index.html fallback and
// no server-side rendering, because Tauri serves it from its own protocol.
//
// The `validation` mode builds the app for the end-to-end validations: it
// compiles the pseudo-locale catalog that `task gen:pseudo-locale` generates
// (ADR-0019) and loads the validation harness (src/routes/+layout.ts).
import { fileURLToPath } from 'node:url';

import { paraglideVitePlugin } from '@inlang/paraglide-js';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import tailwindcss from '@tailwindcss/vite';
import { defineConfig } from 'vite';

export default defineConfig(({ mode }) => {
  const validation = mode === 'validation';
  return {
    clearScreen: false,
    plugins: [
      tailwindcss(),
      sveltekit({
        adapter: adapter({ fallback: 'index.html' }),
        compilerOptions: {
          // Runes mode for every project file; dependencies keep their own mode.
          runes: ({ filename }) => (filename.split(/[/\\]/).includes('node_modules') ? undefined : true),
        },
      }),
      paraglideVitePlugin({ project: validation ? './.pseudo-locale/project.inlang' : './project.inlang' }),
    ],
    resolve: {
      alias: {
        // The browser build of this micromark dependency decodes character
        // references by assigning innerHTML; its default build looks them up
        // in a table, so the Markdown path never creates markup (ADR-0021,
        // decision 7).
        'decode-named-character-reference': fileURLToPath(
          new URL('node_modules/decode-named-character-reference/index.js', import.meta.url),
        ),
      },
    },
    // Tauri expects a fixed port, and the dev server only listens locally.
    server: { host: '127.0.0.1', port: 5173, strictPort: true },
    test: {
      // Tests live in the repository's tests/ folder, which mirrors the code.
      dir: '../../tests/apps/desktop',
      environment: 'node',
      include: ['**/*.test.ts'],
    },
  };
});
