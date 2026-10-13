// @ts-check
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { isAllowed, licenseProblems, parsedReport } from '../../scripts/check-licenses.mjs';

describe('isAllowed', () => {
  it('allows a license from the policy', () => {
    assert.equal(isAllowed('MIT'), true);
    assert.equal(isAllowed('Apache-2.0'), true);
  });

  it('refuses a license outside the policy', () => {
    assert.equal(isAllowed('GPL-3.0-only'), false);
    assert.equal(isAllowed('WTFPL'), false);
  });

  it('needs one allowed alternative of an OR', () => {
    assert.equal(isAllowed('MIT OR GPL-3.0-or-later'), true);
    assert.equal(isAllowed('(Apache-2.0 OR MIT)'), true);
    assert.equal(isAllowed('GPL-2.0-only OR AGPL-3.0-only'), false);
  });

  it('needs every part of an AND', () => {
    assert.equal(isAllowed('(MIT AND Zlib)'), true);
    assert.equal(isAllowed('MIT AND GPL-3.0-only'), false);
  });

  it('reads an exception as part of its license', () => {
    assert.equal(isAllowed('(Apache-2.0 WITH LLVM-exception)'), true);
    assert.equal(isAllowed('GPL-2.0-only WITH Classpath-exception-2.0'), false);
  });

  it('honors parentheses', () => {
    assert.equal(isAllowed('GPL-3.0-only AND (MIT OR Apache-2.0)'), false);
    assert.equal(isAllowed('(GPL-3.0-only AND MIT) OR Apache-2.0'), true);
  });

  it('rejects an expression that is not SPDX', () => {
    assert.throws(() => isAllowed('MIT OR'), /not valid SPDX/);
    assert.throws(() => isAllowed('(MIT'), /unbalanced/);
  });
});

describe('licenseProblems', () => {
  it('lists the packages whose license is not allowed', () => {
    const problems = licenseProblems({
      MIT: [{ license: 'MIT', name: 'allowed-package', versions: ['1.0.0'] }],
      Unknown: [{ license: 'Unknown', name: 'unknown-package', versions: ['2.0.0'] }],
      'GPL-3.0-only': [{ license: 'GPL-3.0-only', name: 'copyleft-package', versions: ['3.0.0', '3.1.0'] }],
    });

    assert.deepEqual(problems, [
      'unknown-package@2.0.0: the license "Unknown" is not allowed (ADR-0017)',
      'copyleft-package@3.0.0,3.1.0: the license "GPL-3.0-only" is not allowed (ADR-0017)',
    ]);
  });

  it('reports an expression that is not SPDX', () => {
    const problems = licenseProblems({
      'MIT OR': [{ license: 'MIT OR', name: 'broken-package', versions: ['1.0.0'] }],
    });

    assert.equal(problems.length, 1);
    assert.match(problems[0] ?? '', /broken-package@1\.0\.0: .*not valid SPDX/);
  });
});

describe('parsedReport', () => {
  it('returns the license map', () => {
    const report = parsedReport('{"MIT":[{"license":"MIT","name":"a","versions":["1.0.0"]}]}');

    assert.deepEqual(Object.keys(report), ['MIT']);
  });

  it("fails with pnpm's message when pnpm reported an error", () => {
    assert.throws(
      () => parsedReport('{"error":{"code":"ERR_PNPM_X","message":"run pnpm install"}}'),
      /pnpm licenses list failed: run pnpm install/,
    );
  });

  it('fails on output that is not a license map', () => {
    assert.throws(() => parsedReport('[]'), /no JSON object/);
    assert.throws(() => parsedReport('{"MIT":"oops"}'), /no package list for MIT/);
  });
});
