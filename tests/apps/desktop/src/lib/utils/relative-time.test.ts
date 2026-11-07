import { describe, expect, it } from 'vitest';

import { relativeTime } from '../../../../../../apps/desktop/src/lib/utils/relative-time.js';

describe('relativeTime', () => {
  const now = 1_000_000_000_000;
  // In the machine's locale, as the app shows it.
  const words = new Intl.RelativeTimeFormat(undefined, { numeric: 'always', style: 'narrow' });

  it('names the largest unit that fits', () => {
    expect(relativeTime(now - 8 * 24 * 60 * 60 * 1000, now)).toBe(words.format(-1, 'week'));
    expect(relativeTime(now - 2 * 60 * 60 * 1000, now)).toBe(words.format(-2, 'hour'));
    expect(relativeTime(now - 30 * 1000, now)).toBe(words.format(-30, 'second'));
  });

  it('says that what just happened is in the past', () => {
    expect(relativeTime(now - 400, now)).toBe(words.format(-0, 'second'));
    expect(relativeTime(now, now)).not.toBe(words.format(0, 'second'));
  });

  it('reads a moment in the future as now', () => {
    expect(relativeTime(now + 5000, now)).toBe(relativeTime(now, now));
  });
});
