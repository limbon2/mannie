import { describe, expect, it } from 'vitest';

import { formatDuration, formatSeconds, secondsUntil } from '../../../../../../apps/desktop/src/lib/utils/durations.js';

describe('secondsUntil', () => {
  it('rounds up to whole seconds and never goes below zero', () => {
    expect(secondsUntil(10_001, 0)).toBe(11);
    expect(secondsUntil(10_000, 0)).toBe(10);
    expect(secondsUntil(0, 5_000)).toBe(0);
  });
});

describe('formatDuration', () => {
  it('keeps a tenth of a second below ten seconds', () => {
    expect(formatDuration(3_240)).toMatch(/3[.,]2/u);
    expect(formatDuration(42_900)).toMatch(/42/u);
    expect(formatDuration(42_900)).not.toMatch(/42[.,]9/u);
  });

  it('names minutes and seconds, and leaves seconds out beyond an hour', () => {
    const minutes = formatDuration(125_000);
    const hours = formatDuration(3_725_000);

    expect(minutes).toMatch(/2/u);
    expect(minutes).toMatch(/5/u);
    expect(hours).toMatch(/1/u);
    expect(hours).toMatch(/2/u);
    expect(hours).not.toMatch(/5/u);
  });

  it('never goes below zero', () => {
    expect(formatDuration(-5)).toMatch(/0/u);
  });
});

describe('formatSeconds', () => {
  it('names the unit in the owner language', () => {
    expect(formatSeconds(5)).toMatch(/5/u);
    expect(formatSeconds(5)).not.toBe('5');
  });
});
