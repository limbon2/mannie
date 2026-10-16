// @ts-check
// The link's contracts only grow (ADR-0002, ADR-0016; docs/features/workers/
// compatibility-and-updates.md). `buf breaking` compares the protobuf files
// with two baselines: the merge base with the base branch, which catches a
// breaking change in a branch or in uncommitted work, and the newest release
// tag that has the files, which catches one that reached the base branch.
// A baseline without the files is skipped, because the contracts start in M4.
// Since M5 the files live in the Workers plugin's folder (ADR-0031), so a
// baseline from before keeps them in `proto/`, and buf compares the folders.
//
// `task lint:proto` runs it from the repository's root. MANNIE_CONTRACTS_BASE
// names the base branch, `origin/main` by default. CI checks out the whole
// history, so that both baselines exist.

import { spawnSync } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

/** The folder of the contracts, which buf.yaml names as its module. */
export const PROTO_FOLDER = 'plugins/workers/proto';

/** Where commits from before M5 keep the contracts. */
export const EARLIER_PROTO_FOLDERS = ['proto'];

/** The base branch when MANNIE_CONTRACTS_BASE names none. */
export const DEFAULT_BASE = 'origin/main';

/** buf's launcher, from the pinned npm package of this repository. */
const BUF = join(dirname(fileURLToPath(import.meta.url)), '..', 'node_modules', '@bufbuild', 'buf', 'bin', 'buf');

/**
 * A commit to compare with, and the folder of its contracts.
 * @typedef {{ name: string, commit: string, contracts: string }} Baseline
 */

/**
 * Runs git in `folder` and returns its output without the final newline, or
 * `undefined` when git fails.
 * @param {string} folder
 * @param {string[]} args
 * @returns {string | undefined}
 */
function git(folder, args) {
  const result = spawnSync('git', args, { cwd: folder, encoding: 'utf8' });
  return result.status === 0 ? result.stdout.trim() : undefined;
}

/**
 * The folder of the contracts in `commit`, or `undefined` when it has none.
 * @param {string} folder the repository
 * @param {string} commit
 * @returns {string | undefined}
 */
export function contractsFolder(folder, commit) {
  return [PROTO_FOLDER, ...EARLIER_PROTO_FOLDERS].find(
    (candidate) => git(folder, ['cat-file', '-e', `${commit}:${candidate}`]) !== undefined,
  );
}

/**
 * Whether `commit` has the contracts.
 * @param {string} folder the repository
 * @param {string} commit
 * @returns {boolean}
 */
export function hasContracts(folder, commit) {
  return contractsFolder(folder, commit) !== undefined;
}

/**
 * The newest release tag, by version, whose commit has the contracts.
 * @param {string} folder the repository
 * @returns {string | undefined}
 */
export function newestReleaseWithContracts(folder) {
  const tags = (git(folder, ['tag', '--list', 'v*', '--sort=-v:refname']) ?? '').split('\n').filter(Boolean);
  return tags.find((tag) => hasContracts(folder, tag));
}

/**
 * The baselines that have the contracts: the merge base with `base`, then
 * the newest release with contracts, each once.
 * @param {string} folder the repository
 * @param {string} base the base branch
 * @returns {{ baselines: Baseline[], skipped: string[] }}
 */
export function baselines(folder, base) {
  const mergeBase = git(folder, ['merge-base', 'HEAD', base]);
  if (mergeBase === undefined) {
    throw new Error(
      `there is no merge base with ${base}; fetch it with its history, or name another base in MANNIE_CONTRACTS_BASE`,
    );
  }
  const candidates = [{ name: `the merge base with ${base}`, commit: mergeBase }];
  const release = newestReleaseWithContracts(folder);
  if (release !== undefined) {
    candidates.push({
      name: `release ${release}`,
      commit: git(folder, ['rev-parse', `${release}^{commit}`]) ?? release,
    });
  }
  /** @type {Baseline[]} */
  const found = [];
  /** @type {string[]} */
  const skipped = [];
  for (const candidate of candidates) {
    const contracts = contractsFolder(folder, candidate.commit);
    if (contracts === undefined) {
      skipped.push(candidate.name);
    } else if (!found.some((baseline) => baseline.commit === candidate.commit)) {
      found.push({ ...candidate, contracts });
    }
  }
  return { baselines: found, skipped };
}

/**
 * Runs `buf breaking` in `folder` against `baseline`, with buf's output on
 * this process's, and returns whether the contracts only grew.
 * @param {string} folder the repository
 * @param {Baseline} baseline
 * @returns {boolean}
 */
export function onlyGrew(folder, baseline) {
  const against = `.git#ref=${baseline.commit},subdir=${baseline.contracts}`;
  const result = spawnSync(process.execPath, [BUF, 'breaking', PROTO_FOLDER, '--against', against], {
    cwd: folder,
    stdio: 'inherit',
  });
  return result.status === 0;
}

const isMain = process.argv[1] === fileURLToPath(import.meta.url);
if (isMain) {
  const folder = process.cwd();
  const base = process.env.MANNIE_CONTRACTS_BASE ?? DEFAULT_BASE;
  try {
    const { baselines: found, skipped } = baselines(folder, base);
    for (const name of skipped) {
      console.log(`check-link-contracts: ${name} has no contracts; nothing to compare with`);
    }
    for (const baseline of found) {
      if (onlyGrew(folder, baseline)) {
        console.log(`check-link-contracts: the contracts only grew since ${baseline.name}`);
      } else {
        console.error(
          `error: the contracts changed incompatibly since ${baseline.name}; they change only by addition (ADR-0002, ADR-0016)`,
        );
        process.exitCode = 1;
      }
    }
  } catch (error) {
    console.error(`error: ${error instanceof Error ? error.message : String(error)}`);
    process.exitCode = 1;
  }
}
