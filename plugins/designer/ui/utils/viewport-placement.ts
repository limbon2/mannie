import { type FloatingLayer, floatingLayers, type LayerRect, watchFloatingLayers } from '@mannie/plugin-sdk/surface';

import type { GraphicsPlacement } from '../generated/bindings.js';

/** The overlap in CSS pixels, with a zero size when the edges do not meet. */
function intersect(a: LayerRect, b: LayerRect): LayerRect {
  const x = Math.max(a.x, b.x);
  const y = Math.max(a.y, b.y);
  const width = Math.max(0, Math.min(a.x + a.width, b.x + b.width) - x);
  const height = Math.max(0, Math.min(a.y + a.height, b.y + b.height) - y);
  return {
    height,
    width,
    x,
    y,
  };
}

function hasArea(rect: LayerRect): boolean {
  return Math.min(rect.width, rect.height) > 0;
}

/** A modal covers the shell. Other floating UI covers the viewport where it overlaps the visible crop. */
export function viewportPlacement(
  bounds: LayerRect,
  crop: LayerRect,
  shown: boolean,
  layers: readonly FloatingLayer[],
  scaleFactor: number,
): GraphicsPlacement {
  const clip = intersect(bounds, crop);
  const covered = layers.some((layer) => {
    const modal = layer.kind === 'dialog';
    const overlaps = hasArea(intersect(clip, layer.rect));
    return modal || overlaps;
  });
  return {
    bounds,
    clip,
    scaleFactor,
    visible: shown && hasArea(clip) && !covered,
  };
}

function clientCrop(ancestors: readonly HTMLElement[]): LayerRect {
  const root = document.documentElement;
  let crop: LayerRect = {
    height: root.clientHeight,
    width: root.clientWidth,
    x: 0,
    y: 0,
  };
  for (const parent of ancestors) {
    const style = getComputedStyle(parent);
    const rect = parent.getBoundingClientRect();
    const clipX = style.overflowX !== 'visible';
    const clipY = style.overflowY !== 'visible';

    crop = intersect(crop, {
      height: clipY ? parent.clientHeight : crop.height,
      width: clipX ? parent.clientWidth : crop.width,
      x: clipX ? rect.x + parent.clientLeft : crop.x,
      y: clipY ? rect.y + parent.clientTop : crop.y,
    });
  }
  return crop;
}

/** Follows a native viewport's placeholder while it is mounted. No frames pass through the webview. */
export function watchViewport(element: HTMLElement, place: (placement: GraphicsPlacement) => void): () => void {
  const ancestors: HTMLElement[] = [];
  let previous = '';
  let frame: number | null = null;
  let trackingTransition = false;
  const measure = (): void => {
    frame = null;
    const { height, width, x, y } = element.getBoundingClientRect();
    const shown = element.isConnected && document.visibilityState === 'visible' && element.closest('[inert]') === null;
    const placement = viewportPlacement(
      {
        height,
        width,
        x,
        y,
      },
      clientCrop(ancestors),
      shown && getComputedStyle(element).visibility === 'visible',
      floatingLayers(),
      window.devicePixelRatio,
    );
    const key = JSON.stringify(placement);
    if (key !== previous) {
      previous = key;
      place(placement);
    }
    if (shown && trackingTransition) {
      trackingTransition = document.getAnimations().some((animation) => {
        const changing = animation instanceof CSSTransition;
        const running = animation.playState === 'running';
        return changing && running;
      });
      if (trackingTransition) {
        schedule();
      }
    }
  };

  function schedule(): void {
    frame ??= requestAnimationFrame(measure);
  }

  function measureNow(): void {
    if (frame !== null) {
      cancelAnimationFrame(frame);
    }
    // WebView2 may suspend animation frames during navigation or a change of visibility.
    measure();
  }

  function transitionStarted(): void {
    trackingTransition = true;
    schedule();
  }

  let density: MediaQueryList;

  function watchDensity(): void {
    density = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
    density.addEventListener('change', watchDensity, { once: true });
    measureNow();
  }

  const resized = new ResizeObserver(measureNow);
  const changed = new MutationObserver((records) => {
    const moved = [element, ...ancestors].some((node, index) => node.parentElement !== (ancestors[index] ?? null));
    if (moved) {
      observeAncestors();
    }
    const attributesChanged = records.some((record) => record.type === 'attributes');
    if (moved || attributesChanged) {
      measureNow();
    }
  });

  function observeAncestors(): void {
    resized.disconnect();
    changed.disconnect();
    ancestors.length = 0;
    for (let parent = element.parentElement; parent !== null; parent = parent.parentElement) {
      ancestors.push(parent);
    }
    for (const node of [element, ...ancestors]) {
      resized.observe(node);
      changed.observe(node, { attributeFilter: ['class', 'style', 'inert', 'hidden'], childList: true });
    }
    // A detached branch may next be inserted under a different pane.
    if (!element.isConnected) {
      changed.observe(document, { childList: true, subtree: true });
    }
  }

  observeAncestors();
  watchDensity();
  const stopLayers = watchFloatingLayers(measureNow);
  document.addEventListener('scroll', schedule, true);
  document.addEventListener('transitionrun', transitionStarted);
  document.addEventListener('visibilitychange', measureNow);
  window.addEventListener('resize', schedule);

  return () => {
    resized.disconnect();
    changed.disconnect();
    stopLayers();
    density.removeEventListener('change', watchDensity);
    document.removeEventListener('scroll', schedule, true);
    document.removeEventListener('transitionrun', transitionStarted);
    document.removeEventListener('visibilitychange', measureNow);
    window.removeEventListener('resize', schedule);
    if (frame !== null) {
      cancelAnimationFrame(frame);
    }
  };
}
