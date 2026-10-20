import assert from 'node:assert/strict';
import { test } from 'node:test';

import { cargoTestArguments } from '../../scripts/ci/run-cargo-test.mjs';

test('a focused Cargo test keeps each argument intact', () => {
  const args = ['test', '--locked', '-p', 'mannie-runtime', '--test', 'tasks', 'one_test', '--', '--exact'];
  assert.deepEqual(cargoTestArguments(JSON.stringify(args)), args);
});

test('a focused nextest run accepts its expression as one argument', () => {
  const args = ['nextest', 'run', '--locked', '--package', 'mannie-runtime', '-E', 'test(one_test) | test(other_test)'];
  assert.deepEqual(cargoTestArguments(JSON.stringify(args)), args);
});

test('invalid workflow argument data fails before starting Cargo', () => {
  for (const input of ['not JSON', '{}', 'null', '[1]', '["test", null]']) {
    assert.throws(() => cargoTestArguments(input));
  }
});

test('a repair cannot accidentally select an entire workspace or another command', () => {
  for (const args of [
    ['clean', '--locked', '-p', 'mannie-runtime', '--exact'],
    ['test', '--locked', '--workspace', '--exact'],
    ['test', '--locked', '-p', 'mannie-runtime'],
    ['test', '-p', 'mannie-runtime', '--exact'],
  ]) {
    assert.throws(() => cargoTestArguments(JSON.stringify(args)), /Select a locked/u);
  }
});
