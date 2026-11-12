// Floating layers of the shell, `@mannie/plugin-sdk/surface` (ADR-0063,
// decision 5): the open dialogs, menus, selects, popovers and tooltips that
// the shell's components draw over the page. A plugin that places a native
// view over its pane reads them, because the view would cover them: the
// webview draws its HTML below every native child window.
//
// The layers come from the markers that bits-ui puts on its content
// elements, so nothing in the shadcn-svelte components changes.

/** What kind of layer floats over the shell. */
export type FloatingLayerKind = 'dialog' | 'menu' | 'popover' | 'select' | 'tooltip';

/** A rectangle in the app webview's CSS pixels. */
export interface LayerRect {
  readonly height: number;
  readonly width: number;
  readonly x: number;
  readonly y: number;
}

/** An open layer and where it is. */
export interface FloatingLayer {
  readonly kind: FloatingLayerKind;
  readonly rect: LayerRect;
}

/** The parts of an element that the tracker reads. */
export interface LayerElement {
  getAttribute(name: string): string | null;
  getBoundingClientRect(): LayerRect;
}

/** Where the tracker looks for layers: the document, or a test's stand-in. */
export interface LayerRoot {
  querySelectorAll(selector: string): Iterable<LayerElement>;
}

/** The bits-ui content markers, by the kind of layer they mark. */
const MARKERS: readonly (readonly [string, FloatingLayerKind])[] = [
  ['data-dialog-content', 'dialog'],
  ['data-alert-dialog-content', 'dialog'],
  ['data-menu-content', 'menu'],
  ['data-dropdown-menu-content', 'menu'],
  ['data-context-menu-content', 'menu'],
  ['data-menubar-content', 'menu'],
  ['data-select-content', 'select'],
  ['data-combobox-content', 'select'],
  ['data-popover-content', 'popover'],
  ['data-link-preview-content', 'popover'],
  ['data-tooltip-content', 'tooltip'],
];

const SELECTOR = MARKERS.map(([marker]) => `[${marker}]`).join(',');

function layerKind(element: LayerElement): FloatingLayerKind | null {
  for (const [marker, kind] of MARKERS) {
    if (element.getAttribute(marker) !== null) {
      return kind;
    }
  }
  return null;
}

/** The layers that are open now, closing ones and empty ones left out. */
export function floatingLayers(root: LayerRoot = document): FloatingLayer[] {
  const layers: FloatingLayer[] = [];
  for (const element of root.querySelectorAll(SELECTOR)) {
    const kind = layerKind(element);
    if (kind === null) {
      continue;
    }
    if (element.getAttribute('data-state') === 'closed') {
      continue;
    }
    const { height, width, x, y } = element.getBoundingClientRect();
    const empty = width <= 0 || height <= 0;
    // A dialog covers the shell whatever its own size.
    const unseen = empty && kind !== 'dialog';
    if (unseen) {
      continue;
    }
    layers.push({ kind, rect: { height, width, x, y } });
  }
  return layers;
}

function sameLayers(left: readonly FloatingLayer[], right: readonly FloatingLayer[]): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

/**
 * Calls `onChange` with the open layers now and after each change, until
 * the returned function is called.
 */
export function watchFloatingLayers(onChange: (layers: FloatingLayer[]) => void): () => void {
  let last: FloatingLayer[] | null = null;
  let frame: number | null = null;
  const read = (): void => {
    frame = null;
    const layers = floatingLayers();
    const changed = last === null || !sameLayers(last, layers);
    if (changed) {
      last = layers;
      onChange(layers);
    }
  };
  const schedule = (): void => {
    frame ??= requestAnimationFrame(read);
  };
  const observer = new MutationObserver(schedule);
  observer.observe(document.body, {
    attributeFilter: ['data-state', 'style', 'hidden'],
    attributes: true,
    childList: true,
    subtree: true,
  });
  window.addEventListener('resize', schedule);
  read();
  return () => {
    observer.disconnect();
    window.removeEventListener('resize', schedule);
    if (frame !== null) {
      cancelAnimationFrame(frame);
    }
  };
}
