// @ts-check
// Cargo's lockfiles (format version 4) and the rules that ADR-0035 sets for
// them (decision 5): the launcher's lock is seeded from the committed locks,
// every package of it that does not come from a path must equal a package of
// a committed lock, and two committed locks agree on every crate that both
// pin within one semver-compatible range.
//
// A lock is a header followed by `[[package]]` tables. Every table starts
// with `name` and `version`, and a package that does not come from a path
// also has `source`, and from a registry `checksum`. The parser keeps each
// table's text, so a seed writes the tables back as Cargo wrote them.

import { readFileSync } from 'node:fs';

import { compareText } from './manifests.mjs';

/**
 * One package of a lock.
 * @typedef {object} LockPackage
 * @property {string} checksum empty for a package that does not come from a registry
 * @property {string} name
 * @property {string} source empty for a package that comes from a path
 * @property {string} text the package's table as the lock writes it, without its header
 * @property {string} version
 */

/**
 * A lock: its header, with the format version, and its packages.
 * @typedef {{ header: string, packages: LockPackage[] }} Lock
 */

/**
 * A committed lock and the path that messages name it by.
 * @typedef {{ lock: Lock, name: string }} NamedLock
 */

/**
 * Crates that committed locks pin within one compatible range at different
 * versions.
 * @typedef {object} Disagreement
 * @property {string} name the crate's name
 * @property {{ checksum: string, lock: string, version: string }[]} pins each lock's pin
 * @property {string} range the compatible range, such as `1` or `0.14`
 * @property {string} source
 */

const PACKAGE_HEADER = '\n[[package]]\n';

/**
 * Parses the text of a lock.
 * @param {string} text
 * @returns {Lock}
 */
export function parseLock(text) {
  const [header, ...tables] = text.replaceAll('\r\n', '\n').split(PACKAGE_HEADER);
  const packages = tables.map((table) => {
    /** @param {string} key */
    const field = (key) => new RegExp(`^${key} = "([^"]*)"$`, 'mu').exec(table)?.[1] ?? '';
    return {
      checksum: field('checksum'),
      name: field('name'),
      source: field('source'),
      text: table.trimEnd(),
      version: field('version'),
    };
  });
  return { header: header.trimEnd(), packages };
}

/**
 * Reads a lock from a file.
 * @param {string} file
 */
export function readLock(file) {
  return parseLock(readFileSync(file, 'utf8'));
}

/**
 * The text of a lock, laid out as Cargo writes it.
 * @param {Lock} lock
 */
export function renderLock(lock) {
  const body = lock.packages.map((entry) => `${PACKAGE_HEADER}${entry.text}\n`).join('');
  return `${lock.header}\n${body}`;
}

/**
 * What identifies a package across locks: its name, version and source.
 * @param {LockPackage} entry
 */
function packageKey(entry) {
  return `${entry.name} ${entry.version} ${entry.source}`;
}

/**
 * Whether a package is pinned by its source, a registry or a Git repository,
 * so that the pin check covers it. Packages from paths are the workspaces' own
 * crates, which Cargo resolves again from their manifests.
 * @param {LockPackage} entry
 */
export function isPinned(entry) {
  return entry.source !== '';
}

/**
 * The seed of the launcher's lock: the union of the committed locks'
 * packages, keyed by name, version and source, with the first lock's package
 * where two locks hold one. Cargo then drops what the launcher does not use
 * and resolves the workspaces' own crates again (`cargo metadata --offline`).
 * @param {Lock[]} locks the root's lock first
 * @returns {Lock}
 */
export function seedLock(locks) {
  /** @type {Map<string, LockPackage>} */
  const merged = new Map();
  for (const lock of locks) {
    for (const entry of lock.packages) {
      const key = packageKey(entry);
      if (!merged.has(key)) {
        merged.set(key, entry);
      }
    }
  }
  const packages = [...merged.values()].sort((a, b) => compareText(packageKey(a), packageKey(b)));
  return { header: locks[0].header, packages };
}

/**
 * The semver-compatible range of a version under Cargo's caret rule: the
 * major from 1.0.0 on, `0.<minor>` below that, and `0.0.<patch>` for 0.0.x.
 * @param {string} version
 */
export function compatibleRange(version) {
  const [major, minor, patch] = version.split(/[.+-]/u).map(Number);
  if (major > 0) {
    return String(major);
  }
  if (minor > 0) {
    return `0.${String(minor)}`;
  }
  return `0.0.${String(patch)}`;
}

/**
 * The packages of a generated lock that no committed lock pins with the same
 * name, version, source and checksum.
 * @param {Lock} generated
 * @param {NamedLock[]} committed
 * @returns {string[]} one problem for each refused package
 */
export function pinProblems(generated, committed) {
  /** @type {Map<string, Set<string>>} */
  const checksums = new Map();
  for (const { lock } of committed) {
    for (const entry of lock.packages.filter(isPinned)) {
      const known = checksums.get(packageKey(entry)) ?? new Set();
      known.add(entry.checksum);
      checksums.set(packageKey(entry), known);
    }
  }
  /** @type {string[]} */
  const problems = [];
  for (const entry of generated.packages.filter(isPinned)) {
    const known = checksums.get(packageKey(entry));
    if (known === undefined) {
      problems.push(`${entry.name} ${entry.version} is in the launcher's lock, but no committed lock pins it`);
    } else if (!known.has(entry.checksum)) {
      problems.push(
        `${entry.name} ${entry.version} has the checksum ${entry.checksum} in the launcher's lock, but the committed locks pin ${[...known].join(', ')}`,
      );
    }
  }
  return problems;
}

/**
 * The crates that two or more committed locks pin within one compatible
 * range at different versions or checksums. One lock never pins two
 * versions of a crate within one range, because Cargo unifies them.
 * @param {NamedLock[]} committed
 * @returns {Disagreement[]}
 */
export function disagreements(committed) {
  /** @type {Map<string, Disagreement>} */
  const ranges = new Map();
  for (const { lock, name: lockName } of committed) {
    for (const entry of lock.packages.filter(isPinned)) {
      const range = compatibleRange(entry.version);
      const key = `${entry.name} ${entry.source} ${range}`;
      const found = ranges.get(key) ?? { name: entry.name, pins: [], range, source: entry.source };
      found.pins.push({ checksum: entry.checksum, lock: lockName, version: entry.version });
      ranges.set(key, found);
    }
  }
  return [...ranges.values()].filter((range) => {
    const variants = new Set(range.pins.map((pin) => `${pin.version} ${pin.checksum}`));
    return variants.size > 1;
  });
}

/**
 * Compares two versions by their numeric parts, a pre-release before its
 * release, as Cargo orders them.
 * @param {string} a
 * @param {string} b
 */
export function compareVersions(a, b) {
  const [aRelease, aPre = ''] = a.split('+')[0].split('-');
  const [bRelease, bPre = ''] = b.split('+')[0].split('-');
  const aParts = aRelease.split('.').map(Number);
  const bParts = bRelease.split('.').map(Number);
  for (let index = 0; index < 3; index += 1) {
    const difference = (aParts[index] ?? 0) - (bParts[index] ?? 0);
    if (difference !== 0) {
      return difference;
    }
  }
  if (aPre === bPre) {
    return 0;
  }
  if (aPre === '') {
    return 1;
  }
  return bPre === '' ? -1 : compareText(aPre, bPre);
}
