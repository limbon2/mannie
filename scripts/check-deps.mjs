// @ts-check
// Runs cargo-deny across every Cargo workspace and the generated launcher
// (`task lint:deps`). By default every advisory category fails the check.
// When MANNIE_DENY_ALLOW_ADVISORY_CHURN=1 is set on a pull request or push
// that does not change Cargo manifests, locks or deny.toml, upstream
// unmaintained and yanked notices are allowed while vulnerabilities,
// licenses, bans and sources remain strictly enforced (ADR-0017).

import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(import.meta.dirname, '..');

/**
 * Extra cargo-deny flags derived from the environment.
 * @param {Record<string, string | undefined>} environment
 * @returns {string[]}
 */
export function denyFlags(environment) {
  if (environment.MANNIE_DENY_ALLOW_ADVISORY_CHURN === '1') {
    return ['--allow', 'unmaintained', '--allow', 'yanked'];
  }
  return [];
}

/**
 * Runs a command in the repository root and exits with its status on failure.
 * @param {string} command
 * @param {string[]} args
 */
function run(command, args) {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit' });
  if (result.error !== undefined) {
    throw result.error;
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

const runsDirectly = process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (runsDirectly) {
  const extra = denyFlags(process.env);
  if (extra.length > 0) {
    console.log(
      'lint:deps: allowing upstream unmaintained/yanked churn (vulnerabilities, licenses, bans and sources remain enforced)',
    );
  }
  run('cargo', ['deny', 'fetch', 'db']);
  run(process.execPath, [
    'scripts/plugins/each.mjs',
    '--',
    'cargo',
    'deny',
    '--manifest-path',
    '{manifest}',
    '--locked',
    '--offline',
    'check',
    '--allow',
    'unused-wrapper',
    '--allow',
    'advisory-not-detected',
    ...extra,
  ]);
  run('cargo', [
    'deny',
    '--manifest-path',
    '.generated/desktop/Cargo.toml',
    '--locked',
    '--offline',
    'check',
    ...extra,
  ]);
}
