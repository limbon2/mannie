// @ts-check
// Writes the changes that `patches/@xterm__xterm@6.0.0.patch` and
// `patches/@xterm__addon-webgl@0.19.0.patch` record (ADR-0053, decision 7),
// so that xterm.js runs under the window's security settings:
//
// - xterm.js puts the CSS of its scrollbar, and the theme and the cell sizes
//   of its DOM renderer, into <style> elements, which the Content Security
//   Policy (`style-src 'self'`) blocks. The change puts the same CSS into a
//   constructable stylesheet of the terminal's document, which the policy
//   allows (M14 validation results, S2). The code only sets each element's
//   `textContent` and calls its `remove()`, so the function that replaces
//   the element returns an object with those two members.
// - xterm.js and its WebGL addon carry VS Code's `KeyCodeUtils`, which
//   assigns `toString` to a plain object. Tauri's `freezePrototype` freezes
//   `Object.prototype` in the window, so that assignment throws, and the
//   terminal never opens. The change defines the property instead, which a
//   frozen prototype allows.
//
// xterm.js ships its builds minified, so the recorded patches repeat whole
// lines and cannot be read. This script is their readable form. Only the
// builds that the app bundles change: `lib/xterm.mjs` and
// `lib/addon-webgl.mjs`.
//
// To change a patch, for example for a new version of xterm.js:
//
//   pnpm --dir apps/desktop patch <package>@<version> --edit-dir <folder> --ignore-existing
//   node scripts/patch-xterm.mjs <folder>
//   pnpm --dir apps/desktop patch-commit <folder>
//
// The script fails when a place no longer matches exactly once, so a new
// version gets checked again (risk R43).
//
// Usage: node scripts/patch-xterm.mjs <folder of the package>

import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

/** The function that the three places call instead of making a style element. */
const STYLE_SHEET = `/*
 * Mannie's patch (ADR-0053, decision 7; scripts/patch-xterm.mjs): the CSS
 * that xterm.js writes into <style> elements goes into a constructable
 * stylesheet of the terminal's document instead, which the app's Content
 * Security Policy allows.
 */
function mannieStyleSheet(document) {
  const sheet = new document.defaultView.CSSStyleSheet();
  document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
  return {
    set textContent(css) {
      sheet.replaceSync(css);
    },
    remove() {
      document.adoptedStyleSheets = document.adoptedStyleSheets.filter((adopted) => adopted !== sheet);
    },
  };
}
`;

/**
 * A place of a build, and what replaces it.
 * @typedef {{ name: string, pattern: RegExp, replacement: string }} Place
 */

/** The assignment of `toString` in VS Code's `KeyCodeUtils`, which a frozen `Object.prototype` refuses. */
const KEY_CODE_TO_STRING = {
  name: "KeyCodeUtils's toString",
  pattern: /\(([\w$]+)=>\{function ([\w$]+)\(([\w$]+)\)\{return ([\w$]+)\.keyCodeToStr\(\3\)\}\1\.toString=\2;/gu,
  replacement:
    '($1=>{function $2($3){return $4.keyCodeToStr($3)}Object.defineProperty($1,"toString",{configurable:!0,enumerable:!0,value:$2,writable:!0});',
};

/** @type {Place[]} */
const XTERM_PLACES = [
  {
    name: "the scrollbar's colors, which every renderer uses",
    pattern:
      /this\._styleElement=(\w+)\.mainDocument\.createElement\("style"\),\w+\.appendChild\(this\._styleElement\),/gu,
    replacement: 'this._styleElement=mannieStyleSheet($1.mainDocument),',
  },
  {
    name: "the DOM renderer's cell sizes",
    pattern:
      /this\._dimensionsStyleElement\|\|\(this\._dimensionsStyleElement=this\._document\.createElement\("style"\),this\._screenElement\.appendChild\(this\._dimensionsStyleElement\)\);/gu,
    replacement: 'this._dimensionsStyleElement||(this._dimensionsStyleElement=mannieStyleSheet(this._document));',
  },
  {
    name: "the DOM renderer's theme",
    pattern:
      /this\._themeStyleElement\|\|\(this\._themeStyleElement=this\._document\.createElement\("style"\),this\._screenElement\.appendChild\(this\._themeStyleElement\)\);/gu,
    replacement: 'this._themeStyleElement||(this._themeStyleElement=mannieStyleSheet(this._document));',
  },
  KEY_CODE_TO_STRING,
];

/**
 * A build with each place replaced once.
 * @param {string} build
 * @param {Place[]} places
 * @returns {string}
 */
function replaced(build, places) {
  let text = build;
  for (const place of places) {
    const found = text.match(place.pattern)?.length ?? 0;
    if (found !== 1) {
      throw new Error(`${place.name}: expected one place, found ${String(found)}`);
    }
    text = text.replace(place.pattern, place.replacement);
  }
  return text;
}

/**
 * xterm.js's build with its style elements replaced, `toString` defined and
 * the function of the stylesheets added.
 * @param {string} build the text of `lib/xterm.mjs`
 * @returns {string}
 */
export function patchedXterm(build) {
  const text = replaced(build, XTERM_PLACES);
  if (text.includes('createElement("style")')) {
    throw new Error('the build makes another style element, which the patch does not cover');
  }
  return STYLE_SHEET + text;
}

/**
 * The WebGL addon's build with `toString` defined.
 * @param {string} build the text of `lib/addon-webgl.mjs`
 * @returns {string}
 */
export function patchedWebgl(build) {
  return replaced(build, [KEY_CODE_TO_STRING]);
}

/** The build that each package's patch changes, and how. */
const PACKAGES = {
  '@xterm/addon-webgl': { file: join('lib', 'addon-webgl.mjs'), patch: patchedWebgl },
  '@xterm/xterm': { file: join('lib', 'xterm.mjs'), patch: patchedXterm },
};

const isMain = process.argv[1] === fileURLToPath(import.meta.url);
if (isMain) {
  const folder = process.argv[2];
  if (folder === undefined) {
    console.error('usage: node scripts/patch-xterm.mjs <folder of the package>');
    process.exit(1);
  }
  /** @type {{ name: string }} */
  const manifest = JSON.parse(readFileSync(join(folder, 'package.json'), 'utf8'));
  const target = Object.entries(PACKAGES).find(([name]) => name === manifest.name)?.[1];
  if (target === undefined) {
    console.error(`error: the script patches ${Object.keys(PACKAGES).join(' and ')}, not ${manifest.name}`);
    process.exit(1);
  }
  const file = join(folder, target.file);
  writeFileSync(file, target.patch(readFileSync(file, 'utf8')));
  console.log(`patch-xterm: patched ${file}`);
}
