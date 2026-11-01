// @ts-check
// Checks that no stylesheet of the desktop app's frontend has a selector that
// puts `*` after a `:has()`, as Tailwind's `group-has-*` variants compile to.
// One such selector makes the webview restyle the whole page on every change
// of the page, even while nothing matches it: 5 to 6 ms for each change on
// the reference machine, such as each row that a terminal redraws
// (docs/roadmap/m14-validation.md; src/app.css keeps the utilities that
// would bring one out of the build).
//
// `task build` and `task e2e:build` run this script after the frontend build.
//
// Usage: node scripts/check-css.mjs

import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/** The characters that open and close a group in which a comma does not end a selector. */
const OPENERS = new Set(['(', '[']);
const CLOSERS = new Set([')', ']']);

/** A `:has()` with up to two levels of parentheses inside, then `*`, with or without a combinator before it. */
const STAR_AFTER_HAS = /:has\((?:[^()]|\((?:[^()]|\([^()]*\))*\))*\)\s*[>+~]?\s*\*/u;

/**
 * The selectors of a selector list, split at its commas outside parentheses
 * and brackets.
 * @param {string} list
 * @returns {string[]}
 */
function selectorsOf(list) {
  const selectors = [];
  let depth = 0;
  let start = 0;
  for (let index = 0; index < list.length; index += 1) {
    const character = list[index] ?? '';
    const isComma = character === ',';
    const isOutsideGroups = depth === 0;
    if (OPENERS.has(character)) {
      depth += 1;
    } else if (CLOSERS.has(character)) {
      depth -= 1;
    } else if (isComma && isOutsideGroups) {
      selectors.push(list.slice(start, index).trim());
      start = index + 1;
    }
  }
  selectors.push(list.slice(start).trim());
  return selectors;
}

/**
 * The selectors of a stylesheet that put `*` after a `:has()`.
 * @param {string} css
 * @returns {string[]}
 */
export function starsAfterHas(css) {
  const withoutComments = css.replaceAll(/\/\*[\s\S]*?\*\//gu, '');
  const found = [];
  // The text before each `{` is a rule's selector list or an at-rule's prelude.
  for (const part of withoutComments.split('{')) {
    const prelude = part.slice(part.lastIndexOf('}') + 1).trim();
    if (prelude.startsWith('@') || !prelude.includes(':has(')) {
      continue;
    }
    found.push(...selectorsOf(prelude).filter((selector) => STAR_AFTER_HAS.test(selector)));
  }
  return found;
}

/**
 * The stylesheets below a folder.
 * @param {string} folder
 * @returns {string[]}
 */
function stylesheetsBelow(folder) {
  return readdirSync(folder, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile() && entry.name.endsWith('.css'))
    .map((entry) => join(entry.parentPath, entry.name));
}

const isMain = process.argv[1] === fileURLToPath(import.meta.url);
if (isMain) {
  const output = resolve(import.meta.dirname, '..', 'apps', 'desktop', '.svelte-kit', 'output', 'client');
  if (!existsSync(output)) {
    console.error(`error: ${output} is missing: build the desktop app's frontend first`);
    process.exit(1);
  }
  const stylesheets = stylesheetsBelow(output);
  let failed = false;
  for (const file of stylesheets) {
    for (const selector of starsAfterHas(readFileSync(file, 'utf8'))) {
      console.error(
        `error: ${relative(output, file)} puts * after a :has(), which makes every change of the page restyle all of it: ${selector}`,
      );
      failed = true;
    }
  }
  if (failed) {
    process.exit(1);
  }
  console.log(`check-css: none of the ${String(stylesheets.length)} stylesheets puts * after a :has()`);
}
