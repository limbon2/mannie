// Where a tab's native page may show (ADR-0064, decision 5). The page is a
// native window above the app's HTML, so it hides under an open dialog, menu,
// select or popover, where the pane shows a still of it, and leaves out the
// parts that a tooltip covers.
import type { FloatingLayer, LayerRect } from '@mannie/plugin-sdk/surface';

/** What the pane asks of the core for one tab's page. */
export interface PagePlacement {
  readonly clip: LayerRect[];
  readonly rect: LayerRect;
  readonly visible: boolean;
}

/** The overlap of two rectangles, or `null` when they do not meet. */
export function intersection(left: LayerRect, right: LayerRect): LayerRect | null {
  const x = Math.max(left.x, right.x);
  const y = Math.max(left.y, right.y);
  const width = Math.min(left.x + left.width, right.x + right.width) - x;
  const height = Math.min(left.y + left.height, right.y + right.height) - y;
  const overlaps = width > 0 && height > 0;
  return overlaps ? { height, width, x, y } : null;
}

/**
 * The placement of a page in `placeholder`, under `layers`. An open dialog
 * or menu anywhere hides it, so the first click goes to the app and closes
 * the menu; a tooltip only cuts out its own rectangle.
 */
export function pagePlacement(placeholder: LayerRect, shown: boolean, layers: readonly FloatingLayer[]): PagePlacement {
  const rect: LayerRect = {
    height: placeholder.height,
    width: placeholder.width,
    x: placeholder.x,
    y: placeholder.y,
  };
  const covered = layers.some((layer) => layer.kind !== 'tooltip');
  const empty = rect.width <= 0 || rect.height <= 0;
  if (!shown || covered || empty) {
    return { clip: [], rect, visible: false };
  }
  const clip = layers
    .filter((layer) => layer.kind === 'tooltip')
    .map((layer) => intersection(rect, layer.rect))
    .filter((overlap) => overlap !== null);
  return { clip, rect, visible: true };
}

/** Whether two placements ask the same of the core. */
export function samePlacement(left: PagePlacement | null, right: PagePlacement): boolean {
  const same = left !== null && JSON.stringify(left) === JSON.stringify(right);
  return same;
}
