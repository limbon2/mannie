import { afterEach, describe, expect, it, vi } from 'vitest';

import { viewportPlacement, watchViewport } from '../../ui/utils/viewport-placement.js';

const bounds = {
  height: 80,
  width: 200,
  x: -20,
  y: 10,
};
const crop = {
  height: 70,
  width: 260,
  x: 0,
  y: 0,
};
const clip = {
  height: 60,
  width: 180,
  x: 0,
  y: 10,
};
const outside = {
  height: 10,
  width: 10,
  x: 180,
  y: 12,
};

describe('Designer viewport placement', () => {
  it('keeps the original scene bounds while clipping to the shell in logical coordinates', () => {
    expect(viewportPlacement(bounds, crop, true, [], 1.5)).toEqual({
      bounds,
      clip,
      scaleFactor: 1.5,
      visible: true,
    });
  });

  it('hides under a modal anywhere, or any other layer overlapping the visible portion', () => {
    expect(viewportPlacement(bounds, crop, true, [{ kind: 'dialog', rect: outside }], 1).visible).toBe(false);
    for (const kind of ['menu', 'popover', 'select', 'tooltip'] as const) {
      expect(viewportPlacement(bounds, crop, true, [{ kind, rect: clip }], 1).visible).toBe(false);
      expect(viewportPlacement(bounds, crop, true, [{ kind, rect: outside }], 1).visible).toBe(true);
    }
  });

  it('hides for an inert main view, a hidden window or a fully clipped viewport', () => {
    expect(viewportPlacement(bounds, crop, false, [], 2).visible).toBe(false);
    expect(viewportPlacement(bounds, outside, true, [], 2).visible).toBe(false);
  });
});

/** The layout and observer delivery needed to exercise a WebView2 with deferred animation frames. */
class LayoutNode {
  readonly clientHeight = 480;
  readonly clientLeft = 0;
  readonly clientTop = 0;
  readonly clientWidth = 640;
  inert = false;

  constructor(public parentElement: LayoutNode | null = null) {}

  get isConnected(): boolean {
    const root = Object.is(this, document.documentElement);
    const attached = this.parentElement?.isConnected === true;
    return root || attached;
  }

  closest(): LayoutNode | null {
    return this.inert ? this : (this.parentElement?.closest() ?? null);
  }

  getBoundingClientRect(): typeof bounds {
    return {
      height: this.clientHeight,
      width: this.clientWidth,
      x: 0,
      y: 0,
    };
  }
}

function layoutPage() {
  const root = new LayoutNode();
  const pane = new LayoutNode(root);
  const element = new LayoutNode(pane);
  const observers = new Set<ObservedChanges>();
  const doc = Object.assign(new EventTarget(), {
    body: root,
    documentElement: root,
    querySelectorAll: () => [],
    visibilityState: 'visible',
  });
  const pageWindow = Object.assign(new EventTarget(), {
    devicePixelRatio: 1.5,
    matchMedia: () => new EventTarget(),
  });
  const frame = vi.fn(() => 7);
  const cancelled = vi.fn();

  class ObservedChanges {
    readonly nodes = new Set<unknown>();

    constructor(readonly notify: (records: readonly Pick<MutationRecord, 'type'>[]) => void) {}

    observe(node: unknown): void {
      this.nodes.add(node);
      observers.add(this);
    }

    disconnect(): void {
      this.nodes.clear();
      observers.delete(this);
    }
  }

  function mutate(node: unknown, type: MutationRecordType): void {
    for (const observer of [...observers]) {
      if (observer.nodes.has(node)) {
        observer.notify([{ type }]);
      }
    }
  }

  vi.stubGlobal('document', doc);
  vi.stubGlobal('window', pageWindow);
  vi.stubGlobal('getComputedStyle', () => ({ overflowX: 'hidden', overflowY: 'hidden', visibility: 'visible' }));
  vi.stubGlobal('requestAnimationFrame', frame);
  vi.stubGlobal('cancelAnimationFrame', cancelled);
  vi.stubGlobal('MutationObserver', ObservedChanges);
  vi.stubGlobal(
    'ResizeObserver',
    class {
      disconnect = vi.fn();
      observe = vi.fn();
    },
  );
  return {
    cancelled,
    doc,
    element,
    frame,
    mutate,
    observers,
    pane,
    root,
  };
}

afterEach(() => vi.unstubAllGlobals());

describe('observing the native placeholder', () => {
  it('places it on mount and restores it on uncover even while animation frames are suspended', () => {
    const { cancelled, doc, element, frame, mutate, observers, pane } = layoutPage();
    const place = vi.fn();
    const close = watchViewport(element as unknown as HTMLElement, place);
    expect(place).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({ visible: true }));
    doc.dispatchEvent(new Event('scroll'));
    expect(frame).toHaveBeenCalledOnce();
    pane.inert = true;
    mutate(pane, 'attributes');
    expect(place).toHaveBeenLastCalledWith(expect.objectContaining({ visible: false }));
    expect(cancelled).toHaveBeenCalledWith(7);
    pane.inert = false;
    mutate(pane, 'attributes');
    expect(place).toHaveBeenLastCalledWith(expect.objectContaining({ visible: true }));
    doc.visibilityState = 'hidden';
    doc.dispatchEvent(new Event('visibilitychange'));
    expect(place).toHaveBeenLastCalledWith(expect.objectContaining({ visible: false }));
    doc.visibilityState = 'visible';
    doc.dispatchEvent(new Event('visibilitychange'));
    expect(place).toHaveBeenCalledTimes(5);
    close();
    expect(observers.size).toBe(0);
    doc.visibilityState = 'hidden';
    doc.dispatchEvent(new Event('visibilitychange'));
    expect(place).toHaveBeenCalledTimes(5);
  });

  it('reconnects to the new parent after the placeholder moves between panes', () => {
    const { doc, element, mutate, pane, root } = layoutPage();
    const place = vi.fn();
    const close = watchViewport(element as unknown as HTMLElement, place);

    element.parentElement = null;
    mutate(pane, 'childList');
    expect(place).toHaveBeenLastCalledWith(expect.objectContaining({ visible: false }));
    const next = new LayoutNode(root);

    element.parentElement = next;
    mutate(doc, 'childList');
    expect(place).toHaveBeenLastCalledWith(expect.objectContaining({ visible: true }));
    next.inert = true;
    mutate(next, 'attributes');
    expect(place).toHaveBeenLastCalledWith(expect.objectContaining({ visible: false }));
    close();
  });
});
