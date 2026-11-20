import { describe, expect, it } from 'vitest';

import { documentPoint, nextSelection } from '../../ui/utils/canvas-selection.js';

const element = {
  id: 'rect',
  hue: 27,
  x: -10,
  y: 20,
  width: 50,
  height: 40,
};
const bounds = {
  x: -5,
  y: 10,
  width: 100,
  height: 80,
};
const clip = {
  x: 10,
  y: 20,
  width: 75,
  height: 60,
};

describe('canvas selection in document coordinates', () => {
  it('converts CSS pixels relative to the full viewport through the inverse view', () => {
    expect(documentPoint({ x: 10, y: 32.5 }, bounds)).toEqual({ x: 15, y: 22.5 });
    const view = { origin: { x: -12.25, y: 100 }, zoom: 2.5 };
    expect(documentPoint({ x: 10, y: 32.5 }, bounds, view)).toEqual({ x: -6.25, y: 109 });
  });

  it('hits the visible part, clears on the background and ignores input outside the crop', () => {
    function click(x: number, y: number, current: string | null): string | null {
      return nextSelection(current, { kind: 'point', client: { x, y }, bounds }, element, clip);
    }

    expect(click(10, 30, null)).toBe('rect');
    expect(click(34.99, 69.99, 'rect')).toBe('rect');
    expect(click(35, 35, 'rect')).toBeNull();
    expect(click(10, 70, 'rect')).toBeNull();
    expect(click(9.99, 32, null)).toBeNull();
    expect(click(85, 60, 'rect')).toBe('rect');
  });

  it('selects from the keyboard, clears with Escape and refuses unavailable geometry', () => {
    expect(nextSelection(null, { kind: 'select' }, element, clip)).toBe('rect');
    expect(nextSelection('rect', { kind: 'select' }, element, clip)).toBe('rect');
    expect(nextSelection('rect', { kind: 'clear' }, element, clip)).toBeNull();
    expect(nextSelection(null, { kind: 'select' }, { ...element, width: null }, clip)).toBeNull();
  });
});
