// @ts-check
// Checks that the desktop app's first bundle holds no xterm.js (ADR-0053,
// decision 7; NFR-PERF-10): the terminal's library loads as chunks of its own
// when the first terminal opens, so the app's start does not change.
//
// The first bundle is what the app loads as it starts: the chunks that the
// Vite manifest of the last build marks as entries, SvelteKit's routes
// included, and every chunk that they import statically. Chunks that only a
// dynamic import reaches are left out. A chunk holds xterm.js when its code
// names the class of xterm.js's hidden field for keys.
//
// `task build` and `task e2e:build` run this script after the frontend build.
//
// Usage: node scripts/check-first-bundle.mjs

import { existsSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/** Text that only xterm.js's code holds: the class of its hidden field for keys. */
export const XTERM_MARKER = 'xterm-helper-textarea';

/** Text that only CodeMirror 6's view bundle holds: its editable content class (ADR-0076). */
export const CODEMIRROR_MARKER = 'cm-content';

/**
 * A chunk of the Vite manifest, by the fields that this script reads.
 * @typedef {{ file: string, imports?: string[], isEntry?: boolean }} ManifestChunk
 */

/**
 * The files of the first bundle: the entries and what they import
 * statically, each once.
 * @param {Record<string, ManifestChunk>} manifest
 * @returns {string[]}
 */
export function firstBundle(manifest) {
  const seen = new Set();
  const waiting = Object.keys(manifest).filter((key) => manifest[key]?.isEntry === true);
  for (let key = waiting.pop(); key !== undefined; key = waiting.pop()) {
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);
    waiting.push(...(manifest[key]?.imports ?? []));
  }
  return [...seen].map((key) => manifest[key]?.file).filter((file) => file !== undefined);
}

/**
 * The files of the first bundle that hold xterm.js.
 * @param {Record<string, ManifestChunk>} manifest
 * @param {(file: string) => string} read the text of an output file
 * @returns {string[]}
 */
export function xtermInFirstBundle(manifest, read) {
  return firstBundle(manifest).filter((file) => read(file).includes(XTERM_MARKER));
}

/**
 * The files of the first bundle that hold CodeMirror 6.
 * @param {Record<string, ManifestChunk>} manifest
 * @param {(file: string) => string} read the text of an output file
 * @returns {string[]}
 */
export function codeMirrorInFirstBundle(manifest, read) {
  return firstBundle(manifest).filter((file) => read(file).includes(CODEMIRROR_MARKER));
}

const isMain = process.argv[1] === fileURLToPath(import.meta.url);
if (isMain) {
  const output = resolve(import.meta.dirname, '..', 'apps', 'desktop', '.svelte-kit', 'output', 'client');
  const manifestFile = join(output, '.vite', 'manifest.json');
  if (!existsSync(manifestFile)) {
    console.error(`error: ${manifestFile} is missing: build the desktop app's frontend first`);
    process.exit(1);
  }
  /** @type {Record<string, ManifestChunk>} */
  const manifest = JSON.parse(readFileSync(manifestFile, 'utf8'));
  const readFile = (/** @type {string} */ file) => readFileSync(join(output, file), 'utf8');
  const xtermFound = xtermInFirstBundle(manifest, readFile);
  const cmFound = codeMirrorInFirstBundle(manifest, readFile);
  for (const file of xtermFound) {
    console.error(`error: ${file} holds xterm.js, which the app's first bundle must not load (NFR-PERF-10)`);
  }
  for (const file of cmFound) {
    console.error(`error: ${file} holds CodeMirror, which the app's first bundle must not load (ADR-0076)`);
  }
  const hasXterm = xtermFound.length > 0;
  const hasCodeMirror = cmFound.length > 0;

  if (hasXterm || hasCodeMirror) {
    process.exit(1);
  }
  console.log(
    `check-first-bundle: the ${String(firstBundle(manifest).length)} chunks of the first bundle hold no xterm.js or CodeMirror`,
  );
}
