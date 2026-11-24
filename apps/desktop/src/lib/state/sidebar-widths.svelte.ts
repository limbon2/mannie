// Per-activity widths of the desktop shell's content sidebar (ADR-0077):
// resolves each navigation entry's resize bounds within safe shell limits,
// clamps effective widths to the current window so the main area never
// collapses, and keeps each activity's width in browser storage.
import type { NavigationEntry, SidebarResizeOptions } from '#lib/types/shell-slots.js';

import { browserStorage } from './sidebar-collapse.svelte.js';

/** Storage key for per-activity content sidebar widths in CSS pixels. */
export const SIDEBAR_WIDTHS_STORAGE_KEY = 'mannie.shell.sidebar-widths';

/** Default content sidebar width in CSS pixels, matching `16rem`. */
export const SIDEBAR_DEFAULT_WIDTH = 256;

/** Default minimum width when an entry sets `sidebarResizable: true`. */
export const SIDEBAR_DEFAULT_MIN_WIDTH = 200;

/** Default maximum width when an entry sets `sidebarResizable: true`. */
export const SIDEBAR_DEFAULT_MAX_WIDTH = 480;

/** Hard floor enforced by the shell regardless of plugin options. */
export const SIDEBAR_SHELL_MIN_WIDTH = 180;

/** Hard ceiling enforced by the shell regardless of plugin options. */
export const SIDEBAR_SHELL_MAX_WIDTH = 560;

/** Width of the fixed icon activity bar (`3rem` = 48 px) reserved beside the content sidebar. */
const ACTIVITY_BAR_WIDTH_PX = 48;

/** Minimum main area width preserved whenever the viewport allows it. */
const MAIN_AREA_MIN_WIDTH_PX = 360;

/** Maximum fraction of the window width the content sidebar may occupy. */
const MAX_VIEWPORT_SHARE = 0.45;

/** Normalized resize bounds in CSS pixels for one resizable activity. */
export interface SidebarResizeBounds {
  readonly defaultWidth: number;
  readonly maxWidth: number;
  readonly minWidth: number;
}

/** Live resize callbacks passed from `Shell` to `ShellSidebar` (ADR-0077). */
export interface SidebarResizeControls {
  readonly maxWidth: number;
  readonly minWidth: number;
  readonly onCommit: (width: number) => void;
  readonly onDraggingChange: (dragging: boolean) => void;
  readonly onPreview: (width: number) => void;
  readonly onReset: () => void;
  readonly width: number;
}

function finiteOrDefault(value: number | undefined, fallback: number): number {
  const isValid = typeof value === 'number' && Number.isFinite(value);
  return isValid ? Math.round(value) : fallback;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, Math.round(value)));
}

/**
 * Resolves the normalized resize bounds for `entry`, or `null` when the entry
 * keeps the fixed default width.
 */
export function resolveSidebarResizeBounds(
  entry: Pick<NavigationEntry, 'sidebarResizable'> | undefined,
): SidebarResizeBounds | null {
  if (entry === undefined) {
    return null;
  }

  const setting = entry.sidebarResizable;
  const isDisabled = setting === undefined || setting === false;

  if (isDisabled) {
    return null;
  }

  const options: SidebarResizeOptions = setting === true ? {} : setting;
  const minWidth = clamp(
    finiteOrDefault(options.minWidth, SIDEBAR_DEFAULT_MIN_WIDTH),
    SIDEBAR_SHELL_MIN_WIDTH,
    SIDEBAR_SHELL_MAX_WIDTH,
  );
  const rawMax = clamp(
    finiteOrDefault(options.maxWidth, SIDEBAR_DEFAULT_MAX_WIDTH),
    SIDEBAR_SHELL_MIN_WIDTH,
    SIDEBAR_SHELL_MAX_WIDTH,
  );
  const maxWidth = Math.max(minWidth, rawMax);
  const defaultWidth = clamp(finiteOrDefault(options.defaultWidth, SIDEBAR_DEFAULT_WIDTH), minWidth, maxWidth);

  return { defaultWidth, maxWidth, minWidth };
}

/**
 * Returns the maximum width allowed for `bounds` in a window of `windowWidth`
 * CSS pixels while preserving space for the activity bar and main area.
 */
export function effectiveSidebarMaxWidth(bounds: SidebarResizeBounds, windowWidth: number): number {
  const hasViewport = Number.isFinite(windowWidth) && windowWidth > 0;
  if (!hasViewport) {
    return bounds.maxWidth;
  }

  const roomForMain = Math.floor(windowWidth - ACTIVITY_BAR_WIDTH_PX - MAIN_AREA_MIN_WIDTH_PX);
  const shareCap = Math.floor(windowWidth * MAX_VIEWPORT_SHARE);
  const viewportLimit = Math.min(roomForMain, shareCap);

  return Math.max(bounds.minWidth, Math.min(bounds.maxWidth, viewportLimit));
}

/** Clamps a desired content sidebar width to `bounds` and the current window width. */
export function clampSidebarWidth(width: number, bounds: SidebarResizeBounds, windowWidth: number): number {
  const effectiveMax = effectiveSidebarMaxWidth(bounds, windowWidth);
  const candidate = Number.isFinite(width) ? width : bounds.defaultWidth;
  return clamp(candidate, bounds.minWidth, effectiveMax);
}

function parseStoredWidths(raw: string | null): Partial<Record<string, number>> {
  if (raw === null) {
    return {};
  }

  try {
    const parsed: unknown = JSON.parse(raw);
    const isRecord = typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed);

    if (!isRecord) {
      return {};
    }

    const result: Partial<Record<string, number>> = {};
    for (const [key, value] of Object.entries(parsed as Record<string, unknown>)) {
      const isPositiveNumber = typeof value === 'number' && Number.isFinite(value) && value > 0;
      if (isPositiveNumber) {
        result[key] = Math.round(value);
      }
    }
    return result;
  } catch {
    return {};
  }
}

/** Stores and restores per-activity content sidebar widths. */
export class SidebarWidths {
  #widths: Partial<Record<string, number>> = $state({});
  readonly #storage: Storage | null;

  constructor(storage: Storage | null) {
    this.#storage = storage;
    this.#widths = parseStoredWidths(this.#readRaw());
  }

  /** Effective clamped content width for `activityId` in a window of `windowWidth` pixels. */
  widthFor(activityId: string, bounds: SidebarResizeBounds, windowWidth: number): number {
    const saved = this.#widths[activityId] ?? bounds.defaultWidth;
    return clampSidebarWidth(saved, bounds, windowWidth);
  }

  /** Updates the in-memory width for `activityId` without writing to storage yet. */
  preview(activityId: string, width: number, bounds: SidebarResizeBounds, windowWidth: number): number {
    const clamped = clampSidebarWidth(width, bounds, windowWidth);
    if (this.#widths[activityId] !== clamped) {
      this.#widths = { ...this.#widths, [activityId]: clamped };
    }
    return clamped;
  }

  /** Updates and persists the width for `activityId`. */
  commit(activityId: string, width: number, bounds: SidebarResizeBounds, windowWidth: number): number {
    const clamped = this.preview(activityId, width, bounds, windowWidth);
    this.#save();
    return clamped;
  }

  /** Resets `activityId` back to its default width and persists the change. */
  reset(activityId: string): void {
    if (this.#widths[activityId] === undefined) {
      return;
    }
    const { [activityId]: _, ...rest } = this.#widths;
    this.#widths = rest;
    this.#save();
  }

  #readRaw(): string | null {
    try {
      return this.#storage?.getItem(SIDEBAR_WIDTHS_STORAGE_KEY) ?? null;
    } catch {
      return null;
    }
  }

  #save(): void {
    try {
      this.#storage?.setItem(SIDEBAR_WIDTHS_STORAGE_KEY, JSON.stringify(this.#widths));
    } catch {
      // A full or blocked storage keeps the resized width for this window only.
    }
  }
}

/** The window's per-activity sidebar widths. */
export const sidebarWidths = new SidebarWidths(browserStorage());
