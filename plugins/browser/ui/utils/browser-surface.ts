// The pane's placeholder for a tab's native page (ADR-0064, decision 5). It
// measures the placeholder and the shell's floating layers and reports the
// page's place to the core, which clamps it to the window, because the app
// webview is untrusted (ADR-0004).
import { call } from '@mannie/plugin-sdk/ipc';
import { type FloatingLayer, watchFloatingLayers } from '@mannie/plugin-sdk/surface';

import { commands } from '../generated/bindings.js';

import { type PagePlacement, pagePlacement, samePlacement } from './browser-geometry.js';

/** What the surface needs to know about its pane. */
export interface SurfaceOptions {
  readonly conversationId: string;
  /** Called with whether an overlay hides the page, so the pane can show a still. */
  readonly onHidden: (hidden: boolean) => void;
  readonly tabId: string;
}

/** The measured placement and current palette sent to the native window. */
interface NativePlacement extends PagePlacement {
  readonly cursorAccent: [number, number, number] | null;
  readonly scale: number;
  readonly tabId: string;
}

/** Resolves the app's CSS color to the sRGB bytes the native overlay draws. */
function readCursorAccent(): [number, number, number] | null {
  const color = getComputedStyle(document.documentElement).getPropertyValue('--primary').trim();
  if (color === '') {
    return null;
  }
  const canvas = document.createElement('canvas');
  canvas.width = 1;
  canvas.height = 1;
  const context = canvas.getContext('2d', { willReadFrequently: true });
  if (context === null) {
    return null;
  }
  // A failed CSS assignment leaves the transparent fallback in place.
  context.fillStyle = 'transparent';
  context.fillStyle = color;
  context.fillRect(0, 0, 1, 1);
  const [red = 0, green = 0, blue = 0, alpha] = context.getImageData(0, 0, 1, 1).data;
  return alpha === 255 ? [red, green, blue] : null;
}

/**
 * Follows `element` and the shell's layers and keeps the tab's page over it.
 * The returned function hides the page and stops.
 */
export function followSurface(element: HTMLElement, options: SurfaceOptions): () => void {
  const { conversationId, onHidden, tabId } = options;
  let layers: readonly FloatingLayer[] = [];
  let shown = true;
  let sent: NativePlacement | null = null;
  let frame: number | null = null;
  let stopped = false;
  let cursorAccent: [number, number, number] | null = null;
  let themeChanged = true;

  const send = (placement: PagePlacement): void => {
    const next = { ...placement, cursorAccent, scale: window.devicePixelRatio, tabId };
    if (samePlacement(sent, next)) {
      return;
    }
    sent = next;
    void call(() => commands.browserPlace({ conversationId, placement: next }));
  };
  const measure = (): void => {
    frame = null;
    if (stopped) {
      return;
    }
    const pageVisible = shown && document.visibilityState === 'visible';
    const placement = pagePlacement(element.getBoundingClientRect(), pageVisible, layers);
    const needsAccent = themeChanged && placement.visible;
    if (needsAccent) {
      cursorAccent = readCursorAccent();
      themeChanged = false;
    }
    onHidden(!placement.visible && layers.some((layer) => layer.kind !== 'tooltip'));
    send(placement);
  };
  const schedule = (): void => {
    frame ??= requestAnimationFrame(measure);
  };

  const resized = new ResizeObserver(schedule);
  resized.observe(element);
  const theme = new MutationObserver(() => {
    themeChanged = true;
    schedule();
  });
  theme.observe(document.documentElement, { attributeFilter: ['class'], attributes: true });
  // A panel that the shell hides, or that leaves the window, takes its page
  // along.
  const visibility = new IntersectionObserver((entries) => {
    shown = entries.at(-1)?.isIntersecting ?? false;
    schedule();
  });
  visibility.observe(element);
  const stopLayers = watchFloatingLayers((current) => {
    layers = current;
    schedule();
  });
  window.addEventListener('resize', schedule);
  document.addEventListener('visibilitychange', schedule);
  // Splitters move the placeholder without resizing it.
  const moved = window.setInterval(schedule, 100);
  schedule();

  return () => {
    stopped = true;
    resized.disconnect();
    theme.disconnect();
    visibility.disconnect();
    stopLayers();
    window.removeEventListener('resize', schedule);
    document.removeEventListener('visibilitychange', schedule);
    window.clearInterval(moved);
    if (frame !== null) {
      cancelAnimationFrame(frame);
    }
    const rect = { height: 0, width: 0, x: 0, y: 0 };
    send({ clip: [], rect, visible: false });
  };
}
