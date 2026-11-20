import { describe, expect, it } from 'vitest';

import { formatGeometryValue, geometryInput } from '../../ui/utils/canvas-geometry.js';

const rect = { x: 64, y: 64, width: 240, height: 160 };

describe('rectangle geometry field values', () => {
  it('reads signed decimal values and round-trips all digits of the formatted field', () => {
    const value = -Math.PI;
    expect(geometryInput(rect, 'x', formatGeometryValue(value))).toEqual({ ...rect, x: value });
    expect(geometryInput(rect, 'x', '+.5')).toEqual({ ...rect, x: 0.5 });
  });

  it.each(['', '  ', '+', '-', '.', '-.', '12.', '3 4', '12pt', 'NaN', '0x2', 'Infinity'])(
    'keeps incomplete or non-decimal text %j out of geometry updates',
    (text) => {
      expect(geometryInput(rect, 'x', text)).toBeNull();
    },
  );

  it('allows exact document edges and one-unit dimensions and refuses a single overflowing edge', () => {
    expect(geometryInput(rect, 'x', '-32768')).toEqual({ ...rect, x: -32768 });
    expect(geometryInput(rect, 'y', '-32768.1')).toBeNull();
    expect(geometryInput(rect, 'width', '32704')).toEqual({ ...rect, width: 32704 });
    expect(geometryInput(rect, 'x', '32529')).toBeNull();
    expect(geometryInput(rect, 'height', '32704.01')).toBeNull();
    expect(geometryInput(rect, 'width', '1')).toEqual({ ...rect, width: 1 });
    expect(geometryInput(rect, 'height', '.999')).toBeNull();
  });
});
