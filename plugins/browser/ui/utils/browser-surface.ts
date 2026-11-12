// The pane's placeholder for a tab's native page (ADR-0063, decision 5). It
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

/**
 * Follows `element` and the shell's layers and keeps the tab's page over it.
 * The returned function hides the page and stops.
 */
export function followSurface(element: HTMLElement, options: SurfaceOptions): () => void {
  const { conversationId, onHidden, tabId } = options;
  let layers: readonly FloatingLayer[] = [];
  let shown = true;
  let sent: PagePlacement | null = null;
  let frame: number | null = null;
  let stopped = false;

  const send = (placement: PagePlacement): void => {
    if (samePlacement(sent, placement)) {
      return;
    }
    sent = placement;
    void call(() =>
      commands.browserPlace({
        conversationId,
        placement: { ...placement, scale: window.devicePixelRatio, tabId },
      }),
    );
  };
  const measure = (): void => {
    frame = null;
    if (stopped) {
      return;
    }
    const pageVisible = shown && document.visibilityState === 'visible';
    const placement = pagePlacement(element.getBoundingClientRect(), pageVisible, layers);
    onHidden(!placement.visible && layers.some((layer) => layer.kind !== 'tooltip'));
    send(placement);
  };
  const schedule = (): void => {
    frame ??= requestAnimationFrame(measure);
  };

  const resized = new ResizeObserver(schedule);
  resized.observe(element);
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
