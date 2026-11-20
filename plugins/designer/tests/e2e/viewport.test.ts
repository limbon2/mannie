import { execFile } from 'node:child_process';
import { join } from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

import { dataDirectory } from '#e2e/app.ts';
import { clickWithText } from '#e2e/chat.ts';
import { chooseTrayItem, graphicsSurfaces, processId, seedConversation } from '#e2e/harness.ts';
import { axeViolations, leaveSettings, openSettings, setPluginEnabled, waitForApp } from '#e2e/page.ts';
import { Key } from 'webdriverio';

const ENTRY = '[data-shell-sidebar="bar"] nav a[href*="view=designer"]';
const ADD = '[data-designer-projects-add]';
const SLIDER = '[data-designer-hue] [role="slider"]';
const PLACEHOLDER = '[data-designer-viewport] > div.absolute';
const CANVAS = '[data-designer-canvas]';
const OUTLINE = [255, 255, 255];
const DOCUMENT_ERROR = '[data-designer-document-error]';
const RED = [255, 0, 0];
const NEAR_RED = [255, 4, 0];
const BACKGROUND = [32, 34, 38];
const SEED = { height: 160, width: 240, x: 64, y: 64 };
const SECOND_RECTANGLE = { height: 118, width: 190, x: 47, y: 87 };
const GEOMETRY_PROJECT = 'Geometry edits';
const GEOMETRY_REFERENCE = 'Geometry reference';
const GEOMETRY_FIELDS = ['x', 'y', 'width', 'height'] as const;
const visibleTestWindow = process.platform === 'win32' && process.env.MANNIE_E2E_SHOW_WINDOW !== undefined;
const readScreen = fileURLToPath(new URL('./support/screen-pixels.ps1', import.meta.url));

type DesktopAction =
  'Click' | 'Edge' | 'Minimized' | 'Offscreen' | 'Ready' | 'Restore' | 'RestorePointer' | 'Sample' | 'Show';

async function desktopWindow(action: DesktopAction, bounds: readonly number[] = []): Promise<string> {
  const pid = await processId();
  const args = ['-NoProfile', '-NonInteractive', '-File', readScreen, String(pid), action, ...bounds.map(String)];
  const result = await promisify(execFile)('powershell', args, { windowsHide: true });
  return result.stdout.trim();
}

function openValidationStore(): DatabaseSync {
  return new DatabaseSync(join(dataDirectory(), 'mannie.sqlite'), { timeout: 5_000 });
}

/** Recreate the empty validation store's original Designer schema before the next process opens it. */
function plantLegacyProject(): void {
  const database = openValidationStore();
  try {
    const version = Number(database.prepare('PRAGMA user_version').get()?.user_version);
    expect(database.prepare("SELECT version FROM store_modules WHERE module = 'designer'").get()).toEqual({
      version: 2,
    });
    expect(database.prepare('SELECT count(*) AS count FROM designer_projects').get()).toEqual({ count: 0 });
    database.exec(`
      BEGIN IMMEDIATE;
      DROP TABLE designer_elements;
      ALTER TABLE designer_projects DROP COLUMN document_revision;
      INSERT INTO designer_projects (id, name, created_at)
      VALUES ('0199e0f2-0000-7000-8000-000000000001', 'First rectangle', 1);
      UPDATE store_modules SET version = 1 WHERE module = 'designer';
      PRAGMA user_version = ${String(version - 1)};
      COMMIT;
    `);
  } finally {
    database.close();
  }
}

function storedElement(name: string): Record<string, unknown> {
  const database = openValidationStore();
  try {
    const statement = database.prepare(`
      SELECT e.id, e.x, e.y, e.width, e.height, e.hue, p.document_revision AS revision
      FROM designer_elements e JOIN designer_projects p ON p.id = e.project_id WHERE p.name = ?
    `);
    const rows = statement.all(name);
    expect(rows).toHaveLength(1);
    const [element] = rows;
    if (!element) {
      throw new Error(`the project ${name} has no saved element`);
    }
    return element;
  } finally {
    database.close();
  }
}

async function createProject(name: string): Promise<void> {
  await $(ADD).click();
  await $('#designer-name').setValue(name);
  await $('[data-designer-name-confirm]').click();
  await $('[data-designer-name-dialog]').waitForExist({ reverse: true });
}

function geometryField(name: keyof typeof SEED): string {
  return `[data-designer-geometry="${name}"]`;
}

async function replaceFocusedValue(value: string): Promise<void> {
  await browser.keys([Key.Ctrl, 'a']);
  await browser.keys(value);
}

function sameColor(left: readonly number[] | null | undefined, right: readonly number[]): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

async function waitForColor(color: readonly number[], rectangle = SEED): Promise<void> {
  await browser.waitUntil(
    async () => {
      const [surface] = await graphicsSurfaces();
      const visible = surface?.visible === true;
      const layout = await browser.execute((selector) => {
        const canvas = document.querySelector(selector)?.getBoundingClientRect();
        return canvas ? [canvas.width / 2, canvas.height / 2] : null;
      }, PLACEHOLDER);
      if (!layout) {
        return false;
      }
      const [x = 0, y = 0] = layout;
      const horizontal = x >= rectangle.x && x < rectangle.x + rectangle.width;
      const vertical = y >= rectangle.y && y < rectangle.y + rectangle.height;
      const inside = horizontal && vertical;
      return visible && sameColor(surface.center, inside ? color : BACKGROUND) && sameColor(surface.corner, BACKGROUND);
    },
    { timeoutMsg: `the native window did not present the expected color ${color.join(', ')}` },
  );
}

async function checkHue(hue: number, color: readonly number[], rectangle = SEED): Promise<void> {
  await expect($(SLIDER)).toHaveAttribute('aria-valuenow', String(hue));
  await waitForColor(color, rectangle);
  if (visibleTestWindow) {
    await checkScreenPixels(color, rectangle);
  } else {
    // In these fixtures the point eight units inside the top-left corner of each
    // rectangle lies outside the other rectangle. The GL sample distinguishes them.
    await withFramebufferCenter(rectangle.x + 8, rectangle.y + 8, async () => {
      await browser.waitUntil(
        async () => {
          const [surface] = await graphicsSurfaces();
          return sameColor(surface?.center, color) && sameColor(surface?.corner, BACKGROUND);
        },
        { timeoutMsg: 'the framebuffer did not present the saved rectangle at its document coordinates' },
      );
    });
  }
}

/** The validation host samples the center of its real framebuffer. Resize the
 * placeholder through the normal placement observer to put this point there. */
async function withFramebufferCenter(x: number, y: number, sample: () => Promise<void>): Promise<void> {
  const previous = await browser.execute(
    (selector, width, height) => {
      const canvas = document.querySelector<HTMLElement>(selector);
      if (!canvas) {
        throw new Error('the Designer viewport is missing');
      }
      const style = { width: canvas.style.width, height: canvas.style.height };
      canvas.style.width = `${String(width)}px`;
      canvas.style.height = `${String(height)}px`;
      return style;
    },
    PLACEHOLDER,
    2 * x,
    2 * y,
  );
  try {
    await checkPlacement();
    await sample();
  } finally {
    await browser.execute(
      (selector, style) => {
        const canvas = document.querySelector<HTMLElement>(selector);
        if (canvas) {
          canvas.style.width = style.width;
          canvas.style.height = style.height;
        }
      },
      PLACEHOLDER,
      previous,
    );
    await checkPlacement();
  }
}

async function rectangleScreenBounds(rectangle = SEED): Promise<number[]> {
  return browser.execute(
    (selector, bounds) => {
      const canvas = document.querySelector(selector)?.getBoundingClientRect();
      if (!canvas) {
        throw new Error('the Designer viewport is missing');
      }
      const croppedRight = canvas.width <= bounds.x + bounds.width + 2;
      const croppedBottom = canvas.height <= bounds.y + bounds.height + 2;
      if (croppedRight || croppedBottom) {
        throw new Error('the complete rectangle must be visible for this desktop pixel check');
      }
      return [canvas.x + bounds.x, canvas.y + bounds.y, bounds.width, bounds.height].map((value) =>
        Math.round(value * devicePixelRatio),
      );
    },
    PLACEHOLDER,
    rectangle,
  );
}

async function clickCanvas(x: number, y: number): Promise<void> {
  await browser.waitUntil(
    () =>
      browser.execute(
        (selector, localX, localY) => {
          const canvas = document.querySelector<HTMLButtonElement>(selector);
          if (!canvas || canvas.disabled) {
            return false;
          }
          const bounds = canvas.getBoundingClientRect();
          return canvas === document.elementFromPoint(bounds.x + localX, bounds.y + localY);
        },
        CANVAS,
        x,
        y,
      ),
    { timeoutMsg: 'the canvas did not become the pointer target after the shell overlay closed' },
  );
  const point = await browser.execute(
    (selector, localX, localY) => {
      const canvas = document.querySelector(selector)?.getBoundingClientRect();
      if (!canvas) {
        throw new Error('the Designer viewport is missing');
      }
      return { x: canvas.x + localX, y: canvas.y + localY, scale: devicePixelRatio };
    },
    PLACEHOLDER,
    x,
    y,
  );
  if (visibleTestWindow) {
    const canvas = $(CANVAS);
    await browser.execute((selector) => {
      const element = document.querySelector<HTMLElement>(selector);
      if (!element) {
        throw new Error('the Designer canvas is missing');
      }
      delete element.dataset.testCanvasClick;
      element.addEventListener(
        'click',
        () => {
          element.dataset.testCanvasClick = 'delivered';
        },
        { once: true },
      );
    }, CANVAS);
    const previous = await desktopWindow('Click', [
      Math.round(point.x * point.scale),
      Math.round(point.y * point.scale),
    ]);
    try {
      // Windows queues SendInput. Moving the pointer back sooner can send its button release to another window.
      await expect(canvas).toHaveAttribute('data-test-canvas-click', 'delivered');
    } finally {
      await desktopWindow('RestorePointer', previous.split(',').map(Number));
    }
    return;
  }
  await browser.performActions([
    {
      type: 'pointer',
      id: 'designer-canvas',
      parameters: { pointerType: 'mouse' },
      actions: [
        { type: 'pointerMove', duration: 0, x: Math.round(point.x), y: Math.round(point.y) },
        { type: 'pointerDown', button: 0 },
        { type: 'pointerUp', button: 0 },
      ],
    },
  ]);
  await browser.releaseActions();
}

async function checkSelectionAnnouncement(expected: string): Promise<void> {
  await browser.waitUntil(
    () =>
      browser.execute(
        (selector, message) => {
          const canvas = document.querySelector(selector);
          const status = document.querySelector('[data-designer-selection][role="status"]');
          if (!canvas || !status) {
            return false;
          }
          const describedBy = canvas.getAttribute('aria-describedby')?.split(/\s+/);
          const announced = status.textContent.trim() === message;
          return describedBy?.includes(status.id) && announced;
        },
        CANVAS,
        expected,
      ),
    { timeoutMsg: `the canvas did not announce "${expected}"` },
  );
}

/** Sample consecutive device pixels across the left edge, away from both corners. */
async function checkSelectionEdge(selected: boolean, rectangle = SEED): Promise<void> {
  const expected = selected ? [BACKGROUND, OUTLINE, OUTLINE, RED] : [BACKGROUND, BACKGROUND, RED, RED];
  await browser.waitUntil(
    async () => {
      const [surface] = await graphicsSurfaces();
      const matches = JSON.stringify(surface?.outlineEdge) === JSON.stringify(expected);
      return surface?.visible && matches;
    },
    { interval: 10, timeoutMsg: 'the native framebuffer did not show the expected selection edge' },
  );
  if (visibleTestWindow) {
    const wanted = expected.map(String).join('/');
    let sampled = '';
    let bounds: number[] = [];
    try {
      await browser.waitUntil(async () => {
        bounds = await rectangleScreenBounds(rectangle);
        sampled = (await desktopWindow('Edge', bounds)).split(/\r?\n/).join('/');
        return sampled === wanted;
      });
    } catch (error) {
      throw new Error(`Expected composed edge ${wanted} at ${bounds.join(', ')}. Saw ${sampled}.`, { cause: error });
    }
  }
}

async function waitForHidden(): Promise<void> {
  await browser.waitUntil(async () => (await graphicsSurfaces()).every((surface) => !surface.visible));
}

async function viewportBounds(): Promise<number[] | null> {
  return browser.execute((selector) => {
    const rect = document.querySelector(selector)?.getBoundingClientRect();
    if (rect === undefined) {
      return null;
    }
    return [rect.x, rect.y, rect.width, rect.height].map((value) => Math.round(value * devicePixelRatio));
  }, PLACEHOLDER);
}

/** The offscreen run checks the GL buffer. The visible Windows run also checks what the compositor displays. */
async function checkScreenPixels(color: readonly number[], rectangle = SEED): Promise<void> {
  if (!visibleTestWindow) {
    return;
  }
  let sampled = '';
  try {
    await browser.waitUntil(async () => {
      const bounds = await rectangleScreenBounds(rectangle);
      sampled = (await desktopWindow('Sample', bounds)).split(/\r?\n/).join(' / ');
      return sampled === [color, color, BACKGROUND, BACKGROUND, BACKGROUND, BACKGROUND].map(String).join(' / ');
    });
  } catch (error) {
    throw new Error(`Expected ${color.join(',')} inside the document edges and charcoal outside. Saw ${sampled}.`, {
      cause: error,
    });
  }
}

async function checkPlacement(): Promise<void> {
  let last = '';
  try {
    await browser.waitUntil(async () => {
      const expected = await viewportBounds();
      const [surface] = await graphicsSurfaces();
      last = JSON.stringify({ expected, surface });
      if (!expected || !surface?.visible) {
        return false;
      }
      const actual = [surface.x, surface.y, surface.width, surface.height];
      return actual.every((value, index) => Math.abs(value - (expected[index] ?? Infinity)) <= 1);
    });
  } catch (error) {
    const minimized = await desktopWindow('Minimized');
    throw new Error(`The native child did not follow the placeholder. Minimized: ${minimized}. ${last}`, {
      cause: error,
    });
  }
}

describe('Designer native viewport', () => {
  let restoreStacking = false;

  async function checkTestWindow(): Promise<void> {
    if (visibleTestWindow) {
      await browser.maximizeWindow();
      restoreStacking = (await desktopWindow('Show')) === 'False';
    } else if (process.platform === 'win32') {
      expect(await desktopWindow('Offscreen')).toBe('True');
    }
  }

  async function restart(): Promise<void> {
    const oldPid = await processId();
    await browser.reloadSession();
    await waitForApp();
    expect(await processId()).not.toBe(oldPid);
    await checkTestWindow();
    await $(ENTRY).click();
  }

  before(async () => {
    await waitForApp();
    await checkTestWindow();
    await setPluginEnabled('designer', true);
    await leaveSettings();
    await $(ENTRY).click();
  });

  beforeEach(async () => {
    if (visibleTestWindow) {
      await desktopWindow('Ready');
    }
  });

  after(async () => {
    if (restoreStacking) {
      await desktopWindow('Restore');
    }
  });

  it('keeps project management available on this platform', async () => {
    await $('[data-designer-projects-empty]').waitForDisplayed();
    if (process.platform === 'win32') {
      const pid = await processId();
      await chooseTrayItem('quit');
      await browser.waitUntil(
        () => {
          try {
            process.kill(pid, 0);
            return false;
          } catch {
            return true;
          }
        },
        { timeoutMsg: 'the validation app must release its SQLite database before the legacy fixture is created' },
      );
      plantLegacyProject();
      await browser.reloadSession();
      await waitForApp();
      await checkTestWindow();
      await $(ENTRY).click();
      await browser.setWindowSize(850, 565);
      await checkHue(0, RED);
      expect(storedElement('First rectangle')).toMatchObject({ ...SEED, hue: 0, revision: 1 });
      const label = await $(SLIDER).getAttribute('aria-labelledby');
      const name = await browser.execute((id) => document.getElementById(id ?? '')?.textContent, label);
      expect(name).toBe('Rectangle color');
      const originalTheme = await browser.execute(() => document.documentElement.classList.contains('dark'));
      try {
        for (const dark of [false, true]) {
          await browser.execute((enabled) => document.documentElement.classList.toggle('dark', enabled), dark);
          expect(await axeViolations()).toEqual([]);
          await waitForColor(RED);
        }
      } finally {
        await browser.execute((enabled) => document.documentElement.classList.toggle('dark', enabled), originalTheme);
      }
    } else {
      await createProject('First rectangle');
      await expect($('[data-designer-viewport-error]')).toHaveText(expect.stringContaining('available on Windows'));
      expect(await graphicsSurfaces()).toHaveLength(0);
      await createProject('Second rectangle');
      const selectedName = await $('[data-designer-projects] [aria-current="true"] [data-content]');
      await expect(selectedName).toBeDisplayed();
      expect(await selectedName.getProperty('textContent')).toBe('Second rectangle');
    }
  });

  it('selects and clears the native rectangle with the pointer and keyboard', async function () {
    if (process.platform !== 'win32') {
      this.skip();
    }
    const original = storedElement('First rectangle');
    const selected = 'Rectangle selected. X 64, Y 64, width 240, height 160.';
    const cleared = 'Nothing selected.';
    const canvas = $(CANVAS);
    await expect(canvas).toHaveAttribute('aria-label', 'Project canvas');
    await browser.execute((selector) => document.querySelector<HTMLElement>(selector)?.focus(), SLIDER);
    await browser.keys(Key.Tab);
    await expect(canvas).toBeFocused();
    const focusOutline = await browser.execute((selector) => {
      const element = document.querySelector(selector);
      return element && getComputedStyle(element).outlineStyle;
    }, '[data-designer-viewport]');
    expect(focusOutline).toBe('solid');
    await browser.keys(Key.Enter);
    await checkSelectionAnnouncement(selected);
    await checkSelectionEdge(true);
    await browser.keys(Key.Escape);
    await expect(canvas).toBeFocused();
    await checkSelectionAnnouncement(cleared);
    await checkSelectionEdge(false);
    await browser.keys(Key.Space);
    await checkSelectionAnnouncement(selected);
    await clickCanvas(8, 8);
    await checkSelectionAnnouncement(cleared);
    await checkSelectionEdge(false);

    const started = performance.now();
    await clickCanvas(SEED.x + SEED.width / 2, SEED.y + SEED.height / 2);
    await checkSelectionEdge(true);
    console.info(
      `Designer click to observed native outline, including driver round trips: ${(performance.now() - started).toFixed(1)} ms`,
    );
    await expect(canvas).toBeFocused();
    await checkSelectionAnnouncement(selected);
    expect(await axeViolations()).toEqual([]);
    await clickCanvas(8, 8);
    await checkSelectionAnnouncement(cleared);
    await checkSelectionEdge(false);
    expect(storedElement('First rectangle')).toEqual(original);
  });

  it('presents live slider changes, yields to overlays and releases replaced surfaces', async function () {
    if (process.platform !== 'win32') {
      this.skip();
    }
    await checkPlacement();
    // The framebuffer probe needs a full 368 × 288 canvas above the geometry fields.
    await browser.setWindowSize(880, 680);
    await checkPlacement();
    await checkHue(0, RED);

    async function checkLiveDrag(): Promise<void> {
      const original = storedElement('First rectangle');
      const points = await browser.execute((selector) => {
        const track = document.querySelector('[data-designer-hue]')?.getBoundingClientRect();
        const thumb = document.querySelector(selector)?.getBoundingClientRect();
        if (!track || !thumb) {
          throw new Error('the hue slider is missing');
        }
        return {
          startX: Math.round(thumb.left + thumb.width / 2),
          x: Math.round(track.left + track.width / 2),
          y: Math.round(track.top + track.height / 2),
        };
      }, SLIDER);
      await browser.performActions([
        {
          type: 'pointer',
          id: 'designer-slider',
          parameters: { pointerType: 'mouse' },
          actions: [
            { type: 'pointerMove', duration: 0, x: points.startX, y: points.y },
            { type: 'pointerDown', button: 0 },
          ],
        },
      ]);
      try {
        const started = performance.now();
        await browser.performActions([
          {
            type: 'pointer',
            id: 'designer-slider',
            parameters: { pointerType: 'mouse' },
            actions: [{ type: 'pointerMove', duration: 0, x: points.x, y: points.y }],
          },
        ]);
        // The button stays down while we verify the frame the driver actually swapped.
        await browser.waitUntil(
          async () => {
            const [surface] = await graphicsSurfaces();
            const visible = surface?.visible === true;
            const changed = surface?.center?.[0] === 0;
            const background = sameColor(surface?.corner, [32, 34, 38]);
            return visible && changed && background;
          },
          { interval: 10, timeoutMsg: 'the native surface center did not present the live drag hue' },
        );
        const observed = performance.now() - started;
        console.info(
          `Designer pointer move to observed native framebuffer change, including WebDriver round trips: ≤ ${observed.toFixed(1)} ms`,
        );
        const [dragged] = await graphicsSurfaces();
        if (!dragged?.center) {
          throw new Error('the native framebuffer has no rectangle sample');
        }
        await checkScreenPixels(dragged.center);
        if (visibleTestWindow) {
          const composed = performance.now() - started;
          console.info(
            `Designer pointer move to observed desktop pixel change, including the external sampler: ≤ ${composed.toFixed(1)} ms`,
          );
        }
        expect(storedElement('First rectangle')).toEqual(original);
      } finally {
        await browser.releaseActions();
      }
      await browser.waitUntil(() => storedElement('First rectangle').revision === Number(original.revision) + 1);
    }

    await withFramebufferCenter(SEED.x + SEED.width / 2, SEED.y + SEED.height / 2, checkLiveDrag);
    await $(SLIDER).click();
    await browser.keys(Key.Home);
    await waitForColor(RED);
    await browser.keys(Key.ArrowRight);
    await checkHue(1, NEAR_RED);
    await browser.keys([Key.Ctrl, 'k']);
    await $('[role="dialog"]').waitForDisplayed();
    await waitForHidden();
    await browser.keys(Key.Escape);
    await checkHue(1, NEAR_RED);

    await $(ADD).click();
    await $('#designer-name').waitForDisplayed();
    await waitForHidden();
    await browser.keys(Key.Escape);
    await checkHue(1, NEAR_RED);
    await clickCanvas(SEED.x + 8, SEED.y + 8);
    await checkSelectionAnnouncement('Rectangle selected. X 64, Y 64, width 240, height 160.');
    await createProject('Second rectangle');
    await checkHue(0, RED);
    await checkSelectionAnnouncement('Nothing selected.');
    await $(SLIDER).click();
    await browser.keys(Key.End);
    await checkHue(360, RED);
    await clickWithText('[data-designer-projects] button', 'First rectangle');
    await checkHue(1, NEAR_RED);
    await checkSelectionAnnouncement('Nothing selected.');
    expect(await graphicsSurfaces()).toHaveLength(1);
    await openSettings('plugins.settings');
    await browser.waitUntil(async () => (await graphicsSurfaces()).length === 0);
    await leaveSettings();
    await $(ENTRY).click();
    await checkHue(1, NEAR_RED);
    const conversation = await seedConversation(1);
    const withPanel = new URL(await browser.getUrl());
    withPanel.searchParams.set('panel', `chat.conversation:${conversation}`);
    await browser.url(withPanel.toString());
    await $('[data-side-panels]').waitForDisplayed();
    await browser.setWindowSize(800, 760);
    await browser.waitUntil(() =>
      browser.execute((selector) => document.querySelector(selector)?.closest('[inert]') != null, PLACEHOLDER),
    );
    await waitForHidden();
    await $('[data-side-panels] [data-conversation-actions]').click();
    await $('[data-dropdown-menu-content]').waitForDisplayed();
    await waitForHidden();
    await browser.keys(Key.Escape);
    await $('[data-side-panel-close]').click();
    await $('[data-side-panels]').waitForExist({ reverse: true });
    // The full navigation that opened the panel selected the newest project again.
    await checkHue(360, RED);
    await clickWithText('[data-designer-projects] button', 'First rectangle');
    await browser.setWindowSize(1050, 740);
    await checkHue(1, NEAR_RED);
  });

  it('previews geometry while typing and keeps invalid and cancelled edits out of the store', async function () {
    if (process.platform !== 'win32') {
      this.skip();
    }
    await createProject(GEOMETRY_REFERENCE);
    await $(SLIDER).waitForEnabled();
    await browser.execute((selector) => document.querySelector<HTMLElement>(selector)?.focus(), SLIDER);
    await browser.keys(Key.ArrowRight);
    await checkHue(1, NEAR_RED);
    await createProject(GEOMETRY_PROJECT);
    await $(SLIDER).waitForEnabled();
    await browser.execute((selector) => document.querySelector<HTMLElement>(selector)?.focus(), SLIDER);
    await browser.keys(Key.End);
    await checkHue(360, RED);
    const original = storedElement(GEOMETRY_PROJECT);
    for (const name of GEOMETRY_FIELDS) {
      await expect($(geometryField(name))).toBeDisabled();
      await expect($(geometryField(name))).toHaveValue('');
    }
    await clickCanvas(SEED.x + 8, SEED.y + 8);
    await checkSelectionAnnouncement('Rectangle selected. X 64, Y 64, width 240, height 160.');
    await browser.keys(Key.Tab);
    const xField = $(geometryField('x'));
    await expect(xField).toBeFocused();

    const moved = { ...SEED, x: SECOND_RECTANGLE.x };

    async function checkPreview(rectangle: typeof SEED): Promise<void> {
      await waitForColor(RED, rectangle);
      await checkSelectionEdge(true, rectangle);
      await checkScreenPixels(RED, rectangle);
      expect(storedElement(GEOMETRY_PROJECT)).toEqual(original);
    }

    async function editX(): Promise<void> {
      await replaceFocusedValue(String(moved.x));
      await checkPreview(moved);
      await replaceFocusedValue('-');
      await expect(xField).toHaveValue('-');
      await checkPreview(SEED);
      await replaceFocusedValue('32768');
      await expect(xField).toHaveValue('32768');
      await browser.keys(Key.Enter);
      await expect(xField).toHaveAttribute('aria-invalid', 'true');
      await expect(xField).toHaveValue('32768');
      const errorId = await xField.getAttribute('aria-describedby');
      const described = await browser.execute((references) => {
        const ids = references?.split(/\s+/) ?? [];
        return ids.some((id) => Boolean(document.getElementById(id)?.textContent.trim()));
      }, errorId);
      expect(described).toBe(true);
      await checkPreview(SEED);
      await replaceFocusedValue(String(moved.x));
      await checkPreview(moved);
      await browser.keys(Key.Escape);
      await expect(xField).toHaveValue(String(SEED.x));
      await expect(xField).toBeFocused();
      await checkPreview(SEED);
    }

    if (visibleTestWindow) {
      await editX();
    } else {
      // This point changes between background and fill as the left edge moves.
      await withFramebufferCenter(moved.x + 8, SEED.y + 40, editX);
    }
    await xField.click();
    await replaceFocusedValue(String(moved.x));
    await $(ADD).click();
    await $('#designer-name').waitForDisplayed();
    await waitForHidden();
    await browser.keys(Key.Escape);
    await expect(xField).toHaveValue(String(SEED.x));
    expect(storedElement(GEOMETRY_PROJECT)).toEqual(original);

    await xField.click();
    await replaceFocusedValue(String(moved.x));
    await clickWithText('[data-designer-projects] button', GEOMETRY_REFERENCE);
    await checkHue(1, NEAR_RED);
    expect(storedElement(GEOMETRY_PROJECT)).toEqual(original);
    await clickWithText('[data-designer-projects] button', GEOMETRY_PROJECT);
    await checkHue(360, RED);
    await checkSelectionAnnouncement('Nothing selected.');
  });

  it('saves each completed geometry field once and restores both projects after reload and restart', async function () {
    if (process.platform !== 'win32') {
      this.skip();
    }
    await clickWithText('[data-designer-projects] button', GEOMETRY_PROJECT);
    await clickCanvas(SEED.x + 8, SEED.y + 8);
    const original = storedElement(GEOMETRY_PROJECT);
    const initialRevision = Number(original.revision);

    async function enterField(name: keyof typeof SEED): Promise<void> {
      await expect($(geometryField(name))).toBeFocused();
      await replaceFocusedValue(String(SECOND_RECTANGLE[name]));
    }

    async function checkRevision(edits: number): Promise<void> {
      await browser.waitUntil(() => storedElement(GEOMETRY_PROJECT).revision === initialRevision + edits);
    }

    const xField = $(geometryField('x'));
    await xField.click();
    await enterField('x');
    const database = openValidationStore();
    try {
      database.exec(
        "CREATE TRIGGER designer_test_refuse_geometry BEFORE UPDATE OF x ON designer_elements BEGIN SELECT RAISE(ABORT, 'test fault'); END",
      );
      try {
        await browser.keys(Key.Enter);
        await $(DOCUMENT_ERROR).waitForDisplayed();
        await expect(xField).toHaveValue(String(SEED.x));
        await checkSelectionEdge(true);
        await checkScreenPixels(RED);
        expect(storedElement(GEOMETRY_PROJECT)).toEqual(original);
      } finally {
        database.exec('DROP TRIGGER designer_test_refuse_geometry');
      }
    } finally {
      database.close();
    }
    await $(`${DOCUMENT_ERROR} button`).click();
    await $(DOCUMENT_ERROR).waitForExist({ reverse: true });
    await checkRevision(1);
    await xField.click();
    await browser.keys(Key.Tab);
    await enterField('y');
    await browser.keys(Key.Tab);
    await checkRevision(2);
    await enterField('width');
    await browser.keys(Key.Enter);
    await checkRevision(3);
    await browser.keys(Key.Tab);
    await enterField('height');
    // This click commits on blur and lands inside the old height, beyond the preview's bottom edge.
    await clickCanvas(SECOND_RECTANGLE.x + 8, SECOND_RECTANGLE.y + SECOND_RECTANGLE.height + 8);
    await checkRevision(4);
    await checkSelectionAnnouncement('Nothing selected.');
    await clickCanvas(SECOND_RECTANGLE.x + 8, SECOND_RECTANGLE.y + 8);
    await checkSelectionAnnouncement('Rectangle selected. X 47, Y 87, width 190, height 118.');
    await checkSelectionEdge(true, SECOND_RECTANGLE);
    const first = storedElement(GEOMETRY_REFERENCE);
    const second = storedElement(GEOMETRY_PROJECT);
    expect(first).toMatchObject({ ...SEED, hue: 1 });
    expect(second).toMatchObject({ ...original, ...SECOND_RECTANGLE, revision: initialRevision + 4 });
    expect(first.id).not.toBe(second.id);

    async function checkBoth(): Promise<void> {
      await clickWithText('[data-designer-projects] button', GEOMETRY_PROJECT);
      await checkHue(360, RED, SECOND_RECTANGLE);
      await clickCanvas(SECOND_RECTANGLE.x + 8, SECOND_RECTANGLE.y + 8);
      for (const name of GEOMETRY_FIELDS) {
        await expect($(geometryField(name))).toHaveValue(String(SECOND_RECTANGLE[name]));
      }
      await clickWithText('[data-designer-projects] button', GEOMETRY_REFERENCE);
      await checkHue(1, NEAR_RED);
      expect(storedElement(GEOMETRY_REFERENCE)).toEqual(first);
      expect(storedElement(GEOMETRY_PROJECT)).toEqual(second);
    }

    await browser.refresh();
    await waitForApp();
    await $(ENTRY).click();
    await checkBoth();
    await restart();
    await checkBoth();
  });

  it('restores the confirmed pixels and retries a refused save and read', async function () {
    if (process.platform !== 'win32') {
      this.skip();
    }
    await clickWithText('[data-designer-projects] button', 'First rectangle');
    const first = storedElement('First rectangle');
    const database = openValidationStore();
    try {
      database.exec(
        "CREATE TRIGGER designer_test_refuse BEFORE UPDATE OF hue ON designer_elements BEGIN SELECT RAISE(ABORT, 'test fault'); END",
      );
      try {
        // Only the keypress should edit the hue. A click near the left endpoint
        // can itself select a new value before the key reaches the thumb.
        await browser.execute((selector) => document.querySelector<HTMLElement>(selector)?.focus(), SLIDER);
        await browser.keys(Key.ArrowRight);
        await $(DOCUMENT_ERROR).waitForDisplayed();
        await checkHue(1, NEAR_RED);
        expect(storedElement('First rectangle')).toEqual(first);
      } finally {
        database.exec('DROP TRIGGER designer_test_refuse');
      }
      await $(`${DOCUMENT_ERROR} button`).click();
      await $(DOCUMENT_ERROR).waitForExist({ reverse: true });
      await checkHue(2, [255, 9, 0]);
      await browser.waitUntil(() => storedElement('First rectangle').hue === 2);

      await clickWithText('[data-designer-projects] button', 'Second rectangle');
      await checkHue(360, RED);
      // In WAL mode a writer lock still allows readers. Make this table temporarily
      // unavailable on the validation database so the next real project read fails.
      database.exec('ALTER TABLE designer_elements RENAME TO designer_test_unavailable');
      try {
        await clickWithText('[data-designer-projects] button', 'First rectangle');
        await $(DOCUMENT_ERROR).waitForDisplayed();
        await checkHue(2, [255, 9, 0]);
      } finally {
        database.exec('ALTER TABLE designer_test_unavailable RENAME TO designer_elements');
      }
      await $(`${DOCUMENT_ERROR} button`).click();
      await $(DOCUMENT_ERROR).waitForExist({ reverse: true });
      await checkHue(2, [255, 9, 0]);
      expect(storedElement('First rectangle')).toMatchObject({
        ...first,
        hue: 2,
        revision: Number(first.revision) + 1,
      });
    } finally {
      database.close();
    }
    await setPluginEnabled('designer', false);
    await browser.waitUntil(async () => (await graphicsSurfaces()).length === 0);
  });
});
