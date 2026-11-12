// Where a tab's native page may show (ADR-0063, decision 5): an open dialog,
// menu, select or popover hides it, and a tooltip cuts out its rectangle.
import { describe, expect, it } from 'vitest';

import { intersection, pagePlacement, samePlacement } from '../../../ui/utils/browser-geometry.js';

const PLACEHOLDER = { height: 400, width: 600, x: 100, y: 50 };

describe('pagePlacement', () => {
  it('shows the page over its placeholder without layers', () => {
    expect(pagePlacement(PLACEHOLDER, true, [])).toEqual({ clip: [], rect: PLACEHOLDER, visible: true });
  });

  it('hides the page under a dialog, menu, select or popover anywhere', () => {
    for (const kind of ['dialog', 'menu', 'select', 'popover'] as const) {
      const far = { height: 10, width: 10, x: 2000, y: 2000 };
      expect(pagePlacement(PLACEHOLDER, true, [{ kind, rect: far }]).visible).toBe(false);
    }
  });

  it('cuts a tooltip out of the page', () => {
    const tooltip = { height: 40, width: 100, x: 650, y: 30 };
    const placement = pagePlacement(PLACEHOLDER, true, [{ kind: 'tooltip', rect: tooltip }]);
    expect(placement.visible).toBe(true);
    expect(placement.clip).toEqual([{ height: 20, width: 50, x: 650, y: 50 }]);
  });

  it('hides the page of a hidden or empty placeholder', () => {
    expect(pagePlacement(PLACEHOLDER, false, []).visible).toBe(false);
    expect(pagePlacement({ ...PLACEHOLDER, width: 0 }, true, []).visible).toBe(false);
  });
});

describe('intersection and samePlacement', () => {
  it('finds no overlap between rectangles that only touch', () => {
    expect(intersection(PLACEHOLDER, { height: 10, width: 10, x: 700, y: 50 })).toBeNull();
  });

  it('compares placements by value', () => {
    const placement = pagePlacement(PLACEHOLDER, true, []);
    expect(samePlacement(null, placement)).toBe(false);
    expect(samePlacement(pagePlacement(PLACEHOLDER, true, []), placement)).toBe(true);
  });
});
