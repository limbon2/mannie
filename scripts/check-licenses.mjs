// @ts-check
// Checks the licenses of the desktop app's npm packages against the policy of
// ADR-0017; cargo-deny does the same for the Rust crates (deny.toml). Every
// package the desktop app depends on is checked, development tools included,
// because the bundle can only contain packages from that tree. The tooling at
// the repository root (documentation, lint and end-to-end tools) never enters
// a binary and is not checked.
//
// `task lint` pipes `pnpm licenses list --json --filter mannie-desktop` into
// this script.

import { text } from 'node:stream/consumers';
import { fileURLToPath } from 'node:url';

/** The licenses of ADR-0017, the same list as deny.toml. */
export const ALLOWED_LICENSES = new Set([
  '0BSD',
  'Apache-2.0',
  // Apache-2.0 with an exception that only adds permissions.
  'Apache-2.0 WITH LLVM-exception',
  'BSD-2-Clause',
  'BSD-3-Clause',
  'BSL-1.0',
  'CC0-1.0',
  'ISC',
  'MIT',
  // Allowed for libraries used unmodified.
  'MPL-2.0',
  'Unicode-3.0',
  'Unlicense',
  'Zlib',
]);

/**
 * Packages whose license field is wrong or missing, with the license that was
 * checked by hand and the reason. None so far.
 * @type {Map<string, { license: string, reason: string }>}
 */
export const REVIEWED_PACKAGES = new Map();

/**
 * Splits an SPDX license expression into tokens: parentheses, operators and
 * license identifiers, where `X WITH Y` stays one identifier.
 * @param {string} expression
 * @returns {string[]}
 */
function tokenize(expression) {
  const words = expression.replaceAll('(', ' ( ').replaceAll(')', ' ) ').split(/\s+/).filter(Boolean);
  /** @type {string[]} */
  const tokens = [];
  for (let index = 0; index < words.length; index += 1) {
    const word = words[index] ?? '';
    const isException = words[index + 1] === 'WITH';
    if (isException) {
      tokens.push(`${word} WITH ${words[index + 2] ?? ''}`);
      index += 2;
    } else {
      tokens.push(word);
    }
  }
  return tokens;
}

/**
 * Whether an SPDX license expression is satisfied by the allowed licenses:
 * one alternative of an OR, and every part of an AND.
 * @param {string} expression
 * @param {Set<string>} allowed
 * @returns {boolean}
 */
export function isAllowed(expression, allowed = ALLOWED_LICENSES) {
  const tokens = tokenize(expression);
  let position = 0;

  /** @returns {boolean} */
  function alternatives() {
    let result = combination();
    while (tokens[position] === 'OR') {
      position += 1;
      const next = combination();
      result = result || next;
    }
    return result;
  }

  /** @returns {boolean} */
  function combination() {
    let result = license();
    while (tokens[position] === 'AND') {
      position += 1;
      const next = license();
      result = result && next;
    }
    return result;
  }

  /** @returns {boolean} */
  function license() {
    const token = tokens[position];
    position += 1;
    if (token === '(') {
      const result = alternatives();
      const closes = tokens[position] === ')';
      if (!closes) {
        throw new Error(`unbalanced parentheses in the license expression ${JSON.stringify(expression)}`);
      }
      position += 1;
      return result;
    }
    const isIdentifier = token !== undefined && !['AND', 'OR', ')'].includes(token);
    if (!isIdentifier) {
      throw new Error(`the license expression ${JSON.stringify(expression)} is not valid SPDX`);
    }
    return allowed.has(token);
  }

  const result = alternatives();
  const isComplete = position === tokens.length;
  if (!isComplete) {
    throw new Error(`the license expression ${JSON.stringify(expression)} is not valid SPDX`);
  }
  return result;
}

/**
 * @typedef {{ name: string, versions: string[], license: string }} LicensedPackage
 * The packages per license, as `pnpm licenses list --json` prints them.
 * @typedef {Record<string, LicensedPackage[]>} LicenseReport
 */

/**
 * What is wrong with one package's license, or `undefined` when it is allowed.
 * @param {string} name
 * @param {string[]} versions
 * @param {string} license
 * @returns {string | undefined}
 */
function packageProblem(name, versions, license) {
  const shownPackage = `${name}@${versions.join(',')}`;
  try {
    return isAllowed(license)
      ? undefined
      : `${shownPackage}: the license ${JSON.stringify(license)} is not allowed (ADR-0017)`;
  } catch (error) {
    return `${shownPackage}: ${String(error)}`;
  }
}

/**
 * The packages whose license the policy does not allow, as messages.
 * @param {LicenseReport} report
 * @returns {string[]}
 */
export function licenseProblems(report) {
  /** @type {string[]} */
  const problems = [];
  for (const [license, packages] of Object.entries(report)) {
    for (const licensedPackage of packages) {
      const reviewed = REVIEWED_PACKAGES.get(licensedPackage.name);
      const problem = packageProblem(licensedPackage.name, licensedPackage.versions, reviewed?.license ?? license);
      if (problem !== undefined) {
        problems.push(problem);
      }
    }
  }
  return problems;
}

/**
 * The report that `pnpm licenses list --json` printed, or pnpm's own error
 * when it printed one instead.
 * @param {string} json
 * @returns {LicenseReport}
 */
export function parsedReport(json) {
  /** @type {unknown} */
  const report = JSON.parse(json);
  const isObject = typeof report === 'object' && report !== null && !Array.isArray(report);
  if (!isObject) {
    throw new Error('pnpm licenses list printed no JSON object');
  }
  const entries = Object.entries(report);
  const pnpmError = entries.find(([key]) => key === 'error')?.[1];
  if (pnpmError !== undefined) {
    const message = /** @type {{ message?: unknown }} */ (pnpmError).message;
    throw new Error(`pnpm licenses list failed: ${typeof message === 'string' ? message : JSON.stringify(pnpmError)}`);
  }
  const malformed = entries.find(([, packages]) => !Array.isArray(packages));
  if (malformed !== undefined) {
    throw new Error(`pnpm licenses list printed no package list for ${malformed[0]}`);
  }
  return /** @type {LicenseReport} */ (report);
}

const isMain = process.argv[1] === fileURLToPath(import.meta.url);
if (isMain) {
  const report = parsedReport(await text(process.stdin));
  const problems = licenseProblems(report);
  for (const problem of problems) {
    console.error(`error: ${problem}`);
  }
  if (problems.length > 0) {
    process.exitCode = 1;
  } else {
    const count = Object.values(report).reduce((total, packages) => total + packages.length, 0);
    console.log(`check-licenses: the licenses of ${String(count)} packages of the desktop app are allowed`);
  }
}
