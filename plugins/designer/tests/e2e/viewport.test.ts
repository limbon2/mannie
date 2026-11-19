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
const DOCUMENT_ERROR = '[data-designer-document-error]';
const RED = [255, 0, 0];
const NEAR_RED = [255, 4, 0];
const BACKGROUND = [32, 34, 38];
const SEED = { height: 160, width: 240, x: 64, y: 64 };
const SECOND_RECTANGLE = { height: 118, width: 190, x: 47, y: 87 };
const visibleTestWindow = process.platform === 'win32' && process.env.MANNIE_E2E_SHOW_WINDOW !== undefined;
const readScreen = fileURLToPath(new URL('./support/screen-pixels.ps1', import.meta.url));

type DesktopAction = 'Click' | 'Offscreen' | 'Restore' | 'Sample' | 'Show';

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
  await browser.waitUntil(
    async () => {
      const expected = await viewportBounds();
      const [surface] = await graphicsSurfaces();
      const missing = expected === null;
      const hidden = surface?.visible !== true;
      if (missing || hidden) {
        return false;
      }
      const actual = [surface.x, surface.y, surface.width, surface.height];
      return actual.every((value, index) => Math.abs(value - (expected[index] ?? Infinity)) <= 1);
    },
    { timeoutMsg: 'the native child did not follow the webview placeholder' },
  );
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

  it('focuses the webview placeholder at the middle of the native rectangle', async function () {
    if (process.platform !== 'win32') {
      this.skip();
    }
    if (visibleTestWindow) {
      const [left = 0, top = 0, width = 0, height = 0] = await rectangleScreenBounds();
      await desktopWindow('Click', [Math.round(left + width / 2), Math.round(top + height / 2)]);
    } else {
      const center = await browser.execute(
        (selector, rectangle) => {
          const viewport = document.querySelector(selector)?.getBoundingClientRect();
          if (!viewport) {
            throw new Error('the Designer viewport is missing');
          }
          return {
            x: Math.round(viewport.x + rectangle.x + rectangle.width / 2),
            y: Math.round(viewport.y + rectangle.y + rectangle.height / 2),
          };
        },
        PLACEHOLDER,
        SEED,
      );
      await browser.performActions([
        {
          type: 'pointer',
          id: 'designer-canvas',
          parameters: { pointerType: 'mouse' },
          actions: [
            { type: 'pointerMove', duration: 0, ...center },
            { type: 'pointerDown', button: 0 },
            { type: 'pointerUp', button: 0 },
          ],
        },
      ]);
      await browser.releaseActions();
    }
    await browser.waitUntil(() =>
      browser.execute((selector) => document.activeElement === document.querySelector(selector), PLACEHOLDER),
    );
  });

  it('presents live slider changes, yields to overlays and releases replaced surfaces', async function () {
    if (process.platform !== 'win32') {
      this.skip();
    }
    await checkPlacement();
    await browser.setWindowSize(880, 560);
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
    await createProject('Second rectangle');
    await checkHue(0, RED);
    await $(SLIDER).click();
    await browser.keys(Key.End);
    await checkHue(360, RED);
    await clickWithText('[data-designer-projects] button', 'First rectangle');
    await checkHue(1, NEAR_RED);
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

  it('restores each saved rectangle after a reload and a process restart', async function () {
    if (process.platform !== 'win32') {
      this.skip();
    }
    // Geometry editing arrives in a later feature. Give the second saved document a
    // different rectangle so drawing the seed unconditionally would fail the pixel checks.
    const database = openValidationStore();
    try {
      database.exec('BEGIN IMMEDIATE');
      const update = database.prepare(`
        UPDATE designer_elements SET x = ?, y = ?, width = ?, height = ?
        WHERE project_id = (SELECT id FROM designer_projects WHERE name = 'Second rectangle')
      `);
      const { x, y, width, height } = SECOND_RECTANGLE;
      expect(update.run(x, y, width, height).changes).toBe(1);
      database.exec(
        "UPDATE designer_projects SET document_revision = document_revision + 1 WHERE name = 'Second rectangle'; COMMIT",
      );
    } finally {
      database.close();
    }
    const first = storedElement('First rectangle');
    const second = storedElement('Second rectangle');
    expect(first).toMatchObject({ ...SEED, hue: 1 });
    expect(second).toMatchObject({ ...SECOND_RECTANGLE, hue: 360 });
    expect(first.id).not.toBe(second.id);

    async function checkBoth(): Promise<void> {
      await clickWithText('[data-designer-projects] button', 'Second rectangle');
      await checkHue(360, RED, SECOND_RECTANGLE);
      await clickWithText('[data-designer-projects] button', 'First rectangle');
      await checkHue(1, NEAR_RED);
      expect(storedElement('First rectangle')).toEqual(first);
      expect(storedElement('Second rectangle')).toEqual(second);
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
      await checkHue(360, RED, SECOND_RECTANGLE);
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
