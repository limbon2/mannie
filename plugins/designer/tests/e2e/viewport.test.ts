import { execFile } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

import { clickWithText } from '#e2e/chat.ts';
import { graphicsSurfaces, processId, seedConversation } from '#e2e/harness.ts';
import { axeViolations, leaveSettings, openSettings, setPluginEnabled, waitForApp } from '#e2e/page.ts';
import { Key } from 'webdriverio';

const ENTRY = '[data-shell-sidebar="bar"] nav a[href*="view=designer"]';
const ADD = '[data-designer-projects-add]';
const SLIDER = '[data-designer-hue] [role="slider"]';
const PLACEHOLDER = '[data-designer-viewport] > div.absolute';
const RED = [255, 0, 0];
const visibleTestWindow = process.platform === 'win32' && process.env.MANNIE_E2E_SHOW_WINDOW !== undefined;
const readScreen = fileURLToPath(new URL('./support/screen-pixels.ps1', import.meta.url));

async function desktopWindow(action: 'Restore' | 'Sample' | 'Show', bounds: readonly number[] = []): Promise<string> {
  const pid = await processId();
  const args = ['-NoProfile', '-NonInteractive', '-File', readScreen, String(pid), action, ...bounds.map(String)];
  const result = await promisify(execFile)('powershell', args, { windowsHide: true });
  return result.stdout.trim();
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

async function waitForColor(color: readonly number[]): Promise<void> {
  await browser.waitUntil(
    async () => {
      const [surface] = await graphicsSurfaces();
      const visible = surface?.visible === true;
      const rectangle = sameColor(surface?.center, color);
      const background = sameColor(surface?.corner, [32, 34, 38]);
      return visible && rectangle && background;
    },
    { timeoutMsg: `the native window did not present the expected color ${color.join(', ')}` },
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
async function checkScreenPixels(color: readonly number[]): Promise<void> {
  if (!visibleTestWindow) {
    return;
  }
  let sampled = '';
  try {
    await browser.waitUntil(async () => {
      const bounds = await viewportBounds();
      if (bounds === null) {
        return false;
      }
      sampled = (await desktopWindow('Sample', bounds)).split(/\r?\n/).join(' / ');
      return sampled === [color.join(','), '32,34,38'].join(' / ');
    });
  } catch (error) {
    throw new Error(`Expected desktop pixels ${color.join(',')} / 32,34,38. Saw ${sampled}.`, { cause: error });
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
  before(async () => {
    await waitForApp();
    if (visibleTestWindow) {
      restoreStacking = (await desktopWindow('Show')) === 'False';
    }
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
    await createProject('First rectangle');
    if (process.platform === 'win32') {
      if (visibleTestWindow) {
        await browser.maximizeWindow();
      }
      await waitForColor(RED);
      await checkScreenPixels(RED);
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
      await expect($('[data-designer-viewport-error]')).toHaveText(expect.stringContaining('available on Windows'));
      expect(await graphicsSurfaces()).toHaveLength(0);
      await createProject('Second rectangle');
      const selectedName = await $('[data-designer-projects] [aria-current="true"] [data-content]');
      await expect(selectedName).toBeDisplayed();
      expect(await selectedName.getProperty('textContent')).toBe('Second rectangle');
    }
  });

  it('presents live slider changes, yields to overlays and releases replaced surfaces', async function () {
    if (process.platform !== 'win32') {
      this.skip();
    }
    await checkPlacement();
    await browser.setWindowSize(1100, 760);
    await checkPlacement();
    const points = await browser.execute(() => {
      const track = document.querySelector('[data-designer-hue]')?.getBoundingClientRect();
      if (track === undefined) {
        throw new Error('the hue slider is missing');
      }
      return { x: Math.round(track.left + track.width / 2), y: Math.round(track.top + track.height / 2) };
    });
    await browser.performActions([
      {
        type: 'pointer',
        id: 'designer-slider',
        parameters: { pointerType: 'mouse' },
        actions: [
          { type: 'pointerMove', duration: 0, x: points.x - 20, y: points.y },
          { type: 'pointerDown', button: 0 },
          { type: 'pointerMove', duration: 80, x: points.x, y: points.y },
        ],
      },
    ]);
    try {
      // The button stays down while we verify the frame the driver actually swapped.
      await browser.waitUntil(async () => {
        const [surface] = await graphicsSurfaces();
        const visible = surface?.visible === true;
        const changed = surface?.center?.[0] === 0;
        const background = sameColor(surface?.corner, [32, 34, 38]);
        return visible && changed && background;
      });
      const [dragged] = await graphicsSurfaces();
      if (!dragged?.center) {
        throw new Error('the native framebuffer has no rectangle sample');
      }
      await checkScreenPixels(dragged.center);
    } finally {
      await browser.releaseActions();
    }
    await $(SLIDER).click();
    await browser.keys(Key.Home);
    await waitForColor(RED);
    await browser.keys(Key.ArrowRight);
    await waitForColor([255, 4, 0]);
    await checkScreenPixels([255, 4, 0]);
    await browser.keys([Key.Ctrl, 'k']);
    await $('[role="dialog"]').waitForDisplayed();
    await waitForHidden();
    await browser.keys(Key.Escape);
    await waitForColor([255, 4, 0]);
    await checkScreenPixels([255, 4, 0]);

    await $(ADD).click();
    await $('#designer-name').waitForDisplayed();
    await waitForHidden();
    await browser.keys(Key.Escape);
    await waitForColor([255, 4, 0]);
    await createProject('Second rectangle');
    await waitForColor(RED);
    await clickWithText('[data-designer-projects] button', 'First rectangle');
    await waitForColor([255, 4, 0]);
    expect(await graphicsSurfaces()).toHaveLength(1);
    await openSettings('plugins.settings');
    await browser.waitUntil(async () => (await graphicsSurfaces()).length === 0);
    await leaveSettings();
    await $(ENTRY).click();
    await waitForColor([255, 4, 0]);
    await browser.refresh();
    await waitForApp();
    await $(ENTRY).click();
    await waitForColor(RED);
    await clickWithText('[data-designer-projects] button', 'First rectangle');
    await waitForColor(RED);
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
    await checkScreenPixels(RED);
    await waitForColor(RED);
    await browser.setWindowSize(1100, 760);
    await setPluginEnabled('designer', false);
    await browser.waitUntil(async () => (await graphicsSurfaces()).length === 0);
  });
});
