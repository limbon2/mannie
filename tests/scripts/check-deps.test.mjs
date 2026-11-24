// @ts-check
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { denyFlags } from '../../scripts/check-deps.mjs';

describe('denyFlags', () => {
  it('enforces all advisory categories by default', () => {
    assert.deepEqual(denyFlags({}), []);
    assert.deepEqual(denyFlags({ MANNIE_DENY_ALLOW_ADVISORY_CHURN: '0' }), []);
  });

  it('allows only unmaintained and yanked churn when opted in', () => {
    assert.deepEqual(denyFlags({ MANNIE_DENY_ALLOW_ADVISORY_CHURN: '1' }), [
      '--allow',
      'unmaintained',
      '--allow',
      'yanked',
    ]);
  });
});
