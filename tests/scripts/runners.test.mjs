// @ts-check
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { resolve } from 'node:path';
import { describe, it } from 'node:test';

const executable = process.platform === 'win32' ? 'powershell.exe' : 'pwsh';
const available = spawnSync(executable, [
  '-NoProfile',
  '-NonInteractive',
  '-Command',
  '$PSVersionTable.PSVersion.Major',
]);
const fixture = resolve(import.meta.dirname, 'runners.ps1');
const missingPowerShell = available.error && 'code' in available.error && available.error.code === 'ENOENT';

describe(
  'disposable runner controller',
  {
    skip: missingPowerShell ? 'PowerShell is not installed on this platform' : false,
  },
  () => {
    for (const testCase of [
      'syntax',
      'labels',
      'scheduling',
      'configuration',
      'queue',
      'isolation',
      'api',
      'recovery',
      'identity',
      'lifecycle',
      'lock',
    ]) {
      it(testCase, { timeout: 30_000 }, () => {
        const result = spawnSync(
          executable,
          ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', fixture, '-TestCase', testCase],
          {
            encoding: 'utf8',
            timeout: 25_000,
          },
        );
        assert.ifError(result.error);
        assert.equal(result.status, 0, result.stdout + result.stderr);
      });
    }
  },
);
