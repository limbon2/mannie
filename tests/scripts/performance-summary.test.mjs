import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { afterEach, test } from 'node:test';

const script = resolve(import.meta.dirname, '../../scripts/ci/summarize-performance.mjs');
const folders = [];

afterEach(() => {
  for (const folder of folders.splice(0)) {
    assert.equal(dirname(resolve(folder)), resolve(tmpdir()));
    assert.match(basename(folder), /^mannie-performance-summary-/u);
    rmSync(folder, { force: true, recursive: true });
  }
});

function run(contents, inActions = true) {
  const folder = mkdtempSync(join(tmpdir(), 'mannie-performance-summary-'));
  folders.push(folder);
  if (contents !== undefined) {
    mkdirSync(join(folder, 'target', 'perf'), { recursive: true });
    writeFileSync(join(folder, 'target', 'perf', 'results.json'), contents);
  }
  const summary = join(folder, 'summary.md');
  writeFileSync(summary, 'Earlier step\n\n');
  const environment = { ...process.env };
  delete environment.GITHUB_STEP_SUMMARY;
  if (inActions) {
    environment.GITHUB_STEP_SUMMARY = summary;
  }
  return {
    ...spawnSync(process.execPath, [script], { cwd: folder, encoding: 'utf8', env: environment }),
    summary: readFileSync(summary, 'utf8'),
  };
}

test('performance measurements survive in both the job log and summary', () => {
  const report = {
    figures: { 'cold-start': { limit: 2000, samples: 1, unit: 'ms', value: 321.5 } },
    measuredAt: '2026-10-20T17:00:00.000Z',
    reportOnly: true,
  };
  const result = run(JSON.stringify(report));
  assert.equal(result.status, 0, result.stderr);
  const expected = `\`\`\`json\n${JSON.stringify(report, undefined, 2)}\n\`\`\``;
  assert.ok(result.stdout.includes(expected));
  assert.ok(result.summary.includes(expected));
  assert.ok(result.summary.startsWith('Earlier step\n\n'));
});

test('a missing report is explained without hiding an earlier test failure', () => {
  const result = run(undefined);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.summary, /did not produce a report/u);
});

test('an invalid report fails the reporting step', () => {
  const result = run('{broken');
  assert.notEqual(result.status, 0);
  assert.equal(result.summary, 'Earlier step\n\n');
});

test('a local run prints the measurements without requiring an Actions summary file', () => {
  const result = run('{"figures":{}}', false);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /"figures": \{\}/u);
  assert.equal(result.summary, 'Earlier step\n\n');
});
