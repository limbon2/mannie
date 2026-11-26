import { describe, expect, it } from 'vitest';

import { formatCertificateFingerprint } from '../../../ui/utils/browser-messages.js';

describe('formatCertificateFingerprint', () => {
  it('formats 64-character hex strings into uppercase colon-separated pairs', () => {
    const raw = 'a1b2c3d4'.repeat(8);
    expect(formatCertificateFingerprint(raw)).toBe(Array.from({ length: 8 }, () => 'A1:B2:C3:D4').join(':'));
  });

  it('leaves non-64-hex inputs unchanged', () => {
    expect(formatCertificateFingerprint('not-a-hash')).toBe('not-a-hash');
  });
});
