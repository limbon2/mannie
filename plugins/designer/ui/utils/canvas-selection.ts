import type { LayerRect } from '@mannie/plugin-sdk/surface';

import type { DocumentElement, GraphicsRect } from '../generated/bindings.js';

interface Point {
  x: number;
  y: number;
}

interface View {
  origin: Point;
  zoom: number;
}

const identity: View = { origin: { x: 0, y: 0 }, zoom: 1 };

export type SelectionInput = { bounds: LayerRect; client: Point; kind: 'point' } | { kind: 'clear' | 'select' };

/** The native scene's inverse transform uses CSS pixels relative to the full viewport (ADR-0075). */
export function documentPoint(client: Point, bounds: LayerRect, view: View = identity): Point {
  return {
    x: view.origin.x + (client.x - bounds.x) / view.zoom,
    y: view.origin.y + (client.y - bounds.y) / view.zoom,
  };
}

/** Generated floats permit null on the wire. Only finite geometry can be drawn or hit. */
export function hasGeometry(rect: GraphicsRect): rect is LayerRect {
  return [rect.x, rect.y, rect.width, rect.height].every(Number.isFinite);
}

function contains(rect: LayerRect, point: Point): boolean {
  const withinWidth = point.x >= rect.x && point.x < rect.x + rect.width;
  const withinHeight = point.y >= rect.y && point.y < rect.y + rect.height;
  return withinWidth && withinHeight;
}

export function nextSelection(
  current: string | null,
  input: SelectionInput,
  element: DocumentElement,
  clip: GraphicsRect,
): string | null {
  const clear = input.kind === 'clear';
  if (clear || !hasGeometry(element)) {
    return null;
  }
  if (input.kind === 'point') {
    if (!hasGeometry(clip) || !contains(clip, input.client)) {
      return current;
    }
    return contains(element, documentPoint(input.client, input.bounds)) ? element.id : null;
  }
  return element.id;
}
