// @ts-check
// Paraglide options for the CLI (`task gen`) and the Vite plugin (ADR-0019).
// The locale comes from the core's settings through the `custom-core`
// strategy (src/lib/state/locale.ts); when the owner chose none, the
// operating system's language decides when a catalog exists for it, and
// English otherwise.
import { defineConfig } from '@inlang/paraglide-js';

export default defineConfig({
  emitTsDeclarations: true,
  outdir: './src/lib/paraglide',
  strategy: ['custom-core', 'preferredLanguage', 'baseLocale'],
});
