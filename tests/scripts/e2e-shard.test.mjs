// @ts-check
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { test } from 'node:test';

import { selectShardSpecs, shardOf } from '../e2e/support/shard.ts';

const root = resolve(import.meta.dirname, '../..');

test('no value runs every spec', () => {
  assert.equal(shardOf(undefined), undefined);
  assert.equal(shardOf(''), undefined);
  assert.equal(shardOf('  '), undefined);
});

test('a part of the specs is named by its number and the number of parts', () => {
  assert.deepEqual(shardOf('1/2'), { current: 1, total: 2 });
  assert.deepEqual(shardOf(' 3/3 '), { current: 3, total: 3 });
  assert.deepEqual(shardOf('1/1'), { current: 1, total: 1 });
  assert.deepEqual(shardOf('2/12'), { current: 2, total: 12 });
});

test('a value that names no part stops the run', () => {
  for (const value of ['0/2', '3/2', '1/0', '1', '1/2/3', 'a/b', '-1/2', '1.5/2', '1 / 2', 'half']) {
    assert.throws(() => shardOf(value), /MANNIE_E2E_SHARD must name a part of the specs/u, value);
  }
});

test('the jobs of the CI workflow run every part of the specs', () => {
  const workflow = readFileSync(join(root, '.github', 'workflows', 'ci.yml'), 'utf8');
  const line = workflow.split('\n').find((candidate) => candidate.trimStart().startsWith('shard: '));
  assert.ok(line, 'the end-to-end job has a list of parts');
  // The parts of a full run and the one part of a repair's one spec.
  const lists = [...line.matchAll(/'(?<list>\[[^']+\])'/gu)].map((match) => JSON.parse(match.groups?.list ?? '[]'));
  assert.ok(lists.length > 0, line);
  for (const parts of lists) {
    const total = parts.length;
    assert.deepEqual(
      parts.map(shardOf),
      parts.map((_, index) => ({ current: index + 1, total })),
      line,
    );
  }
});

test('selectShardSpecs partitions specs completely and balances heavy plugin specs across shards', () => {
  const specs = [
    'tests/e2e/specs/chat.test.ts',
    'tests/e2e/specs/chat-locale.test.ts',
    'tests/e2e/specs/child-agents.test.ts',
    'tests/e2e/specs/projects.test.ts',
    'tests/e2e/specs/shell.test.ts',
    'tests/e2e/specs/tools.test.ts',
    'plugins/browser/tests/e2e/browser.test.ts',
    'plugins/git/tests/e2e/git.test.ts',
    'plugins/mcp/tests/e2e/mcp.test.ts',
    'plugins/ssh/tests/e2e/ssh.test.ts',
    'plugins/terminal/tests/e2e/terminal.test.ts',
    'plugins/workers/tests/e2e/workers.test.ts',
  ];
  const first = selectShardSpecs(specs, { current: 1, total: 2 });
  const second = selectShardSpecs(specs, { current: 2, total: 2 });

  assert.deepEqual([...first, ...second].sort(), [...specs].sort());
  assert.equal(first.length + second.length, specs.length);
  assert.ok(first.some((spec) => spec.startsWith('plugins/')));
  assert.ok(second.some((spec) => spec.startsWith('plugins/')));
});
