// @ts-check
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { after, describe, it } from 'node:test';
import { fileURLToPath } from 'node:url';

import { baselines, hasContracts, newestReleaseWithContracts } from '../../scripts/check-link-contracts.mjs';

const script = join(dirname(fileURLToPath(import.meta.url)), '..', '..', 'scripts', 'check-link-contracts.mjs');

const BUF_YAML = 'version: v2\nmodules:\n  - path: proto\nbreaking:\n  use:\n    - FILE\n';

/** @type {string[]} */
const folders = [];

after(() => {
  for (const folder of folders) {
    rmSync(folder, { force: true, recursive: true });
  }
});

/**
 * Runs git in `folder` as a test author and returns its output.
 * @param {string} folder
 * @param {...string} args
 * @returns {string}
 */
function git(folder, ...args) {
  const result = spawnSync(
    'git',
    [
      '-c',
      'user.name=Test',
      '-c',
      'user.email=test@example.com',
      '-c',
      'commit.gpgsign=false',
      '-c',
      'tag.gpgsign=false',
      ...args,
    ],
    { cwd: folder, encoding: 'utf8' },
  );
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim();
}

/**
 * A repository on `main` whose first commit has no contracts.
 * @returns {string}
 */
function repository() {
  const folder = mkdtempSync(join(tmpdir(), 'mannie-contracts-'));
  folders.push(folder);
  git(folder, 'init', '--quiet', '--initial-branch=main');
  writeFileSync(join(folder, 'README.md'), 'A repository before its contracts.\n');
  git(folder, 'add', '.');
  git(folder, 'commit', '--quiet', '-m', 'Start');
  return folder;
}

/**
 * Writes a contract whose message has `fields`.
 * @param {string} folder
 * @param {string} fields
 */
function writeContracts(folder, fields) {
  const contracts = join(folder, 'proto', 'mannie', 'link', 'example', 'v1');
  mkdirSync(contracts, { recursive: true });
  writeFileSync(join(folder, 'buf.yaml'), BUF_YAML);
  writeFileSync(
    join(contracts, 'example.proto'),
    `syntax = "proto3";\n\npackage mannie.link.example.v1;\n\nmessage Example {\n${fields}}\n`,
  );
}

/**
 * Commits everything.
 * @param {string} folder
 * @param {string} message
 */
function commit(folder, message) {
  git(folder, 'add', '.');
  git(folder, 'commit', '--quiet', '-m', message);
}

/**
 * Runs the check in `folder` with `main` as the base branch.
 * @param {string} folder
 */
function check(folder) {
  return spawnSync(process.execPath, [script], {
    cwd: folder,
    encoding: 'utf8',
    env: { ...process.env, MANNIE_CONTRACTS_BASE: 'main' },
  });
}

const NAME = '  string name = 1;\n';
const COUNT = '  uint32 count = 2;\n';

describe('check-link-contracts', () => {
  it('skips a baseline without contracts', () => {
    const folder = repository();
    writeContracts(folder, NAME);

    const result = check(folder);

    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /the merge base with main has no contracts/);
  });

  it('passes an addition and refuses a removal since the merge base', () => {
    const folder = repository();
    writeContracts(folder, NAME);
    commit(folder, 'Add the contracts');

    writeContracts(folder, NAME + COUNT);
    const added = check(folder);
    writeContracts(folder, COUNT);
    const removed = check(folder);

    assert.equal(added.status, 0, added.stdout + added.stderr);
    assert.match(added.stdout, /only grew since the merge base with main/);
    assert.notEqual(removed.status, 0);
    assert.match(removed.stderr, /changed incompatibly since the merge base with main/);
  });

  it('refuses a removal that reached the base branch since the newest release', () => {
    const folder = repository();
    writeContracts(folder, NAME);
    commit(folder, 'Add the contracts');
    git(folder, 'tag', 'v0.4.0');
    writeContracts(folder, COUNT);
    commit(folder, 'Remove a field');

    const result = check(folder);

    assert.notEqual(result.status, 0);
    assert.match(result.stdout, /only grew since the merge base with main/);
    assert.match(result.stderr, /changed incompatibly since release v0\.4\.0/);
  });
});

describe('newestReleaseWithContracts', () => {
  it('takes the newest version whose commit has the contracts', () => {
    const folder = repository();
    git(folder, 'tag', 'v0.3.0');
    writeContracts(folder, NAME);
    commit(folder, 'Add the contracts');
    git(folder, 'tag', 'v0.4.0');
    git(folder, 'tag', 'v0.10.0');

    assert.equal(newestReleaseWithContracts(folder), 'v0.10.0');
    assert.equal(hasContracts(folder, 'v0.3.0'), false);
    assert.equal(hasContracts(folder, 'v0.4.0'), true);
  });
});

describe('baselines', () => {
  it('compares a commit once when the merge base is the release', () => {
    const folder = repository();
    writeContracts(folder, NAME);
    commit(folder, 'Add the contracts');
    git(folder, 'tag', 'v0.4.0');

    const { baselines: found, skipped } = baselines(folder, 'main');

    assert.deepEqual(
      found.map((baseline) => baseline.name),
      ['the merge base with main'],
    );
    assert.deepEqual(skipped, []);
  });

  it('names the base when it has no merge base', () => {
    const folder = repository();

    assert.throws(() => baselines(folder, 'origin/main'), /no merge base with origin\/main/);
  });
});
