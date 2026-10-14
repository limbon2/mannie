import { describe, expect, it } from 'vitest';

import { formatSeconds, secondsUntil } from '../../../../../../apps/desktop/src/lib/utils/durations.js';

describe('secondsUntil', () => {
  it('rounds up to whole seconds and never goes below zero', () => {
    expect(secondsUntil(10_001, 0)).toBe(11);
    expect(secondsUntil(10_000, 0)).toBe(10);
    expect(secondsUntil(0, 5_000)).toBe(0);
  });
});

describe('formatSeconds', () => {
  it('names the unit in the owner language', () => {
    expect(formatSeconds(5)).toMatch(/5/u);
    expect(formatSeconds(5)).not.toBe('5');
  });
});
