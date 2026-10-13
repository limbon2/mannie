// @ts-check
// Generates the pseudo-locale for the desktop app (ADR-0019): an inlang
// project in apps/desktop/.pseudo-locale/ with every catalog of
// apps/desktop/messages/ plus `en-XA`, a copy of the English catalog in which
// every message is marked and lengthened. Validation builds compile it
// (`vite build --mode validation`), so the end-to-end tests can find visible
// text that bypasses the catalog and layouts that break under longer text.
//
// Run it with `task gen:pseudo-locale`. The folder is generated and ignored by
// Git.

import { copyFileSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

/** The pseudo-locale's tag: English, in a private-use region. */
export const PSEUDO_LOCALE = 'en-XA';

/** Marks the start and the end of every pseudo-localized message. */
export const START_MARK = '⟦';
export const END_MARK = '⟧';

/** How much longer than English a message becomes, as translations often are. */
const GROWTH = 0.4;

/** @type {Record<string, string>} */
const ACCENTED = {
  A: 'Å',
  B: 'Ɓ',
  C: 'Ç',
  D: 'Đ',
  E: 'É',
  F: 'Ƒ',
  G: 'Ĝ',
  H: 'Ĥ',
  I: 'Î',
  J: 'Ĵ',
  K: 'Ķ',
  L: 'Ļ',
  M: 'Ṁ',
  N: 'Ñ',
  O: 'Ö',
  P: 'Þ',
  Q: 'Ǫ',
  R: 'Ŕ',
  S: 'Š',
  T: 'Ţ',
  U: 'Û',
  V: 'Ṽ',
  W: 'Ŵ',
  X: 'Ẋ',
  Y: 'Ý',
  Z: 'Ž',
  a: 'á',
  b: 'ƀ',
  c: 'ç',
  d: 'đ',
  e: 'é',
  f: 'ƒ',
  g: 'ĝ',
  h: 'ĥ',
  i: 'î',
  j: 'ĵ',
  k: 'ķ',
  l: 'ļ',
  m: 'ɱ',
  n: 'ñ',
  o: 'ö',
  p: 'þ',
  q: 'ǫ',
  r: 'ŕ',
  s: 'š',
  t: 'ţ',
  u: 'û',
  v: 'ṽ',
  w: 'ŵ',
  x: 'ẋ',
  y: 'ý',
  z: 'ž',
};

/** Variables such as `{name}` and escaped braces stay as they are. */
const PROTECTED = /(\\[{}]|\{[^{}]*\})/;

/**
 * Pseudo-localizes one message pattern: letters are accented, the text grows
 * by about 40%, and the result is wrapped in the start and end marks.
 * @param {string} pattern
 * @returns {string}
 */
export function pseudoLocalize(pattern) {
  let letters = 0;
  const accented = pattern
    .split(PROTECTED)
    .map((part) => {
      if (PROTECTED.test(part)) {
        return part;
      }
      return [...part]
        .map((character) => {
          const replacement = ACCENTED[character];
          if (replacement === undefined) {
            return character;
          }
          letters += 1;
          return replacement;
        })
        .join('');
    })
    .join('');
  const padding = '·'.repeat(Math.max(1, Math.ceil(letters * GROWTH)));
  return `${START_MARK}${accented}${padding}${END_MARK}`;
}

/**
 * Pseudo-localizes a message of the inlang message format: a plain pattern, or
 * an array of variant declarations whose `match` values are patterns.
 * @param {unknown} message
 * @returns {unknown}
 */
export function pseudoLocalizeMessage(message) {
  if (typeof message === 'string') {
    return pseudoLocalize(message);
  }
  if (Array.isArray(message)) {
    return message.map((variant) => {
      const isVariantList = typeof variant === 'object' && variant !== null && 'match' in variant;
      if (!isVariantList) {
        return variant;
      }
      const matches = Object.entries(/** @type {{ match: Record<string, unknown> }} */ (variant).match);
      return {
        ...variant,
        match: Object.fromEntries(matches.map(([key, value]) => [key, pseudoLocalizeMessage(value)])),
      };
    });
  }
  const isObject = typeof message === 'object' && message !== null;
  if (isObject) {
    return Object.fromEntries(Object.entries(message).map(([key, value]) => [key, pseudoLocalizeMessage(value)]));
  }
  return message;
}

/**
 * Pseudo-localizes a catalog: every message except the `$schema` key.
 * @param {Record<string, unknown>} catalog
 * @returns {Record<string, unknown>}
 */
export function pseudoLocalizeCatalog(catalog) {
  return Object.fromEntries(
    Object.entries(catalog).map(([key, value]) => [key, key === '$schema' ? value : pseudoLocalizeMessage(value)]),
  );
}

/**
 * Writes the pseudo-locale project for an app folder.
 * @param {string} app The app folder, which holds project.inlang/ and messages/.
 */
export function generatePseudoLocaleProject(app) {
  const output = join(app, '.pseudo-locale');
  const project = join(output, 'project.inlang');
  const messages = join(output, 'messages');
  rmSync(output, { force: true, recursive: true });
  mkdirSync(project, { recursive: true });
  mkdirSync(messages, { recursive: true });

  const settings = JSON.parse(readFileSync(join(app, 'project.inlang', 'settings.json'), 'utf8'));
  const locales = /** @type {string[]} */ (settings.locales);
  settings.locales = [...locales.filter((locale) => locale !== PSEUDO_LOCALE), PSEUDO_LOCALE];
  // Plugin paths are relative to the folder that holds the project.
  settings.modules = /** @type {string[]} */ (settings.modules).map((module) =>
    module.startsWith('./') ? `../${module.slice(2)}` : module,
  );
  writeFileSync(join(project, 'settings.json'), `${JSON.stringify(settings, undefined, 2)}\n`);
  copyFileSync(join(app, 'project.inlang', 'paraglide.config.js'), join(project, 'paraglide.config.js'));

  for (const file of readdirSync(join(app, 'messages'))) {
    copyFileSync(join(app, 'messages', file), join(messages, file));
  }
  const base = JSON.parse(readFileSync(join(app, 'messages', `${settings.baseLocale}.json`), 'utf8'));
  writeFileSync(
    join(messages, `${PSEUDO_LOCALE}.json`),
    `${JSON.stringify(pseudoLocalizeCatalog(base), undefined, 2)}\n`,
  );
}

const isMain = process.argv[1] === fileURLToPath(import.meta.url);
if (isMain) {
  const app = join(dirname(fileURLToPath(import.meta.url)), '..', 'apps', 'desktop');
  generatePseudoLocaleProject(app);
  console.log(`pseudo-locale: wrote ${join(app, '.pseudo-locale')}`);
}
