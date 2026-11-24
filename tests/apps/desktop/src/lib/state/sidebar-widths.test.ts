// Tests per-activity shell sidebar resize bounds, viewport clamping and
// storage recovery (ADR-0077).
import { describe, expect, it } from 'vitest';

import {
  clampSidebarWidth,
  effectiveSidebarMaxWidth,
  resolveSidebarResizeBounds,
  SIDEBAR_DEFAULT_MAX_WIDTH,
  SIDEBAR_DEFAULT_MIN_WIDTH,
  SIDEBAR_DEFAULT_WIDTH,
  SIDEBAR_SHELL_MAX_WIDTH,
  SIDEBAR_SHELL_MIN_WIDTH,
  SIDEBAR_WIDTHS_STORAGE_KEY,
  SidebarWidths,
} from '../../../../../../apps/desktop/src/lib/state/sidebar-widths.svelte.js';

function fakeStorage(initial: Record<string, string> = {}): Storage {
  const entries = new Map<string, string>(Object.entries(initial));
  return {
    clear: () => {
      entries.clear();
    },
    getItem: (key: string) => entries.get(key) ?? null,
    key: (index: number) => [...entries.keys()][index] ?? null,
    get length() {
      return entries.size;
    },
    removeItem: (key: string) => {
      entries.delete(key);
    },
    setItem: (key: string, value: string) => {
      entries.set(key, value);
    },
  };
}

describe('resolveSidebarResizeBounds', () => {
  it('returns null when resizing is omitted or disabled', () => {
    expect(resolveSidebarResizeBounds(undefined)).toBeNull();
    expect(resolveSidebarResizeBounds({})).toBeNull();
    expect(resolveSidebarResizeBounds({ sidebarResizable: false })).toBeNull();
  });

  it('uses shell defaults for sidebarResizable: true and clamps custom plugin bounds', () => {
    expect(resolveSidebarResizeBounds({ sidebarResizable: true })).toEqual({
      defaultWidth: SIDEBAR_DEFAULT_WIDTH,
      maxWidth: SIDEBAR_DEFAULT_MAX_WIDTH,
      minWidth: SIDEBAR_DEFAULT_MIN_WIDTH,
    });

    expect(
      resolveSidebarResizeBounds({
        sidebarResizable: { defaultWidth: 900, maxWidth: 999, minWidth: 50 },
      }),
    ).toEqual({
      defaultWidth: SIDEBAR_SHELL_MAX_WIDTH,
      maxWidth: SIDEBAR_SHELL_MAX_WIDTH,
      minWidth: SIDEBAR_SHELL_MIN_WIDTH,
    });

    expect(
      resolveSidebarResizeBounds({
        sidebarResizable: { defaultWidth: 100, maxWidth: 220, minWidth: 300 },
      }),
    ).toEqual({
      defaultWidth: 300,
      maxWidth: 300,
      minWidth: 300,
    });
  });
});

describe('clampSidebarWidth and viewport limits', () => {
  it('clamps to window share and preserves main-area room on narrow windows', () => {
    const bounds = { defaultWidth: 256, maxWidth: 480, minWidth: 200 };
    expect(effectiveSidebarMaxWidth(bounds, 1400)).toBe(480);
    expect(effectiveSidebarMaxWidth(bounds, 800)).toBe(360);
    expect(effectiveSidebarMaxWidth(bounds, 620)).toBe(212);
    expect(clampSidebarWidth(500, bounds, 800)).toBe(360);
    expect(clampSidebarWidth(120, bounds, 1200)).toBe(200);
  });
});

describe('SidebarWidths persistence', () => {
  it('persists widths independently per activity, resets and recovers from corrupt storage', () => {
    const storage = fakeStorage();
    const bounds = { defaultWidth: 256, maxWidth: 480, minWidth: 200 };
    const widths = new SidebarWidths(storage);

    expect(widths.widthFor('chat', bounds, 1200)).toBe(256);
    expect(widths.commit('chat', 320, bounds, 1200)).toBe(320);
    expect(widths.commit('network', 410, bounds, 1200)).toBe(410);

    const reloaded = new SidebarWidths(storage);
    expect(reloaded.widthFor('chat', bounds, 1200)).toBe(320);
    expect(reloaded.widthFor('network', bounds, 1200)).toBe(410);

    reloaded.reset('chat');
    expect(reloaded.widthFor('chat', bounds, 1200)).toBe(256);
    expect(reloaded.widthFor('network', bounds, 1200)).toBe(410);

    const corrupt = new SidebarWidths(fakeStorage({ [SIDEBAR_WIDTHS_STORAGE_KEY]: 'not json' }));
    expect(corrupt.widthFor('network', bounds, 1200)).toBe(256);
  });
});
