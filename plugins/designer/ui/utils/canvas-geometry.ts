import type { LayerRect } from '@mannie/plugin-sdk/surface';

import type { GraphicsRect } from '../generated/bindings.js';

import { hasGeometry } from './canvas-selection.js';

export type GeometryField = keyof LayerRect;

export interface GeometryEdit {
  field: GeometryField;
  value: number;
}

export const DOCUMENT_LIMIT = 32_768;
const numbers = new Intl.NumberFormat(undefined, {
  maximumSignificantDigits: 17,
  numberingSystem: 'latn',
  useGrouping: false,
});
const symbols = numbers.formatToParts(-1.5);
const decimal = symbols.find((part) => part.type === 'decimal')?.value ?? '.';
const minus = symbols.find((part) => part.type === 'minusSign')?.value ?? '-';

export function formatGeometryValue(value: number): string {
  return numbers
    .formatToParts(value)
    .filter((part) => part.type !== 'literal')
    .map((part) => part.value)
    .join('');
}

/** Keep the IPC payload limited to the rectangle; a document element also carries its ID and hue. */
export function geometryOf(rectangle: GraphicsRect): LayerRect | null {
  if (!hasGeometry(rectangle)) {
    return null;
  }
  const { height, width, x, y } = rectangle;
  return { height, width, x, y };
}

export function sameGeometry(left: LayerRect, right: LayerRect): boolean {
  const positionMatches = left.x === right.x && left.y === right.y;
  const sizeMatches = left.width === right.width && left.height === right.height;
  return positionMatches && sizeMatches;
}

/** Designer ADR-0002 bounds both edges of each axis, independently of the visible viewport. */
export function geometryInput(base: LayerRect, field: GeometryField, text: string): LayerRect | null {
  const normalized = text.trim().replace(minus, '-').replace(decimal, '.');
  if (!/^[+-]?(?:\d+|\d*\.\d+)$/.test(normalized)) {
    return null;
  }
  const next = { ...base, [field]: Number(normalized) };
  const { height, width, x, y } = next;
  const sizeAllowed = width >= 1 && height >= 1;
  const originAllowed = x >= -DOCUMENT_LIMIT && y >= -DOCUMENT_LIMIT;
  const farCornerAllowed = x + width <= DOCUMENT_LIMIT && y + height <= DOCUMENT_LIMIT;
  return sizeAllowed && originAllowed && farCornerAllowed ? next : null;
}
