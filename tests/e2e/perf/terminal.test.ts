// The terminal's performance budgets (NFR-PERF-06 to 09, docs/product/
// goals-and-scope.md §4; plugins/terminal/docs/README.md, performance),
// measured in the real app with the plugin Terminal on and its compatible
// renderer, whose rows are in the page, so the page can see each change.
// The suite enforces them on the reference machine and under the low-end
// profile (`MANNIE_PERF_PROFILE=low-end`, support/low-end.ts).
// `MANNIE_PERF_REPORT_ONLY=1` reports the figures without failing. Every
// figure is also written to target/perf/terminal.json for the validation
// record, docs/roadmap/m14-validation.md.
import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { Key } from 'webdriverio';

import { repository } from '../support/app.ts';
import { addProfile, sendMessage, startNewConversation, waitForCompleteReply } from '../support/chat.ts';
import { deleteProviderProfiles, processId } from '../support/harness.ts';
import { applyLowEndProfile, cpuMilliseconds, LOW_END } from '../support/low-end.ts';
import { leaveSettings, openSettings, setPluginEnabled, waitForApp } from '../support/page.ts';
import { MODELS, type ProviderServer, startProviderServer } from '../support/provider-server.ts';

const REPORT_ONLY = process.env.MANNIE_PERF_REPORT_ONLY === '1';

/** The rows of the terminal that the compatible renderer draws in the page. */
const ROWS = '[data-terminal-view] .xterm-rows';

/** Terminals that stay open and idle for NFR-PERF-08. */
const IDLE_TERMINALS = 8;

/** How long the idle terminals are measured. */
const IDLE_MILLIS = 60_000;

/** How long a terminal floods for NFR-PERF-07. */
const FLOOD_MILLIS = 10_000;

/** A command that prints without end, in the default shell of this computer. */
const FLOOD = process.platform === 'win32' ? "while ($true) { 'y' }" : 'yes';

interface Figure {
  readonly limit: number;
  readonly samples: number;
  readonly unit: string;
  readonly value: number;
}

declare global {
  interface Window {
    /** Times from a key to the terminal's row showing it. */
    echoLatencies?: number[];
    /** Times from input in the message box to the next painted frame, during a flood. */
    floodLatencies?: number[];
    /** Ends the sampling during a flood. */
    floodSamplingStopped?: boolean;
    /** When the terminal's rows last changed. */
    lastRowChange?: number;
    /** The longest task of the page's main thread during a flood, when the webview reports long tasks. */
    longestTask?: number;
  }
}

const figures: Record<string, Figure> = {};

function percentile(values: readonly number[], fraction: number): number {
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * fraction) - 1)] ?? Number.NaN;
}

/** Records a figure and, unless the run only reports, checks it against its target. */
function check(name: string, value: number, limit: number, unit: string, samples = 1): void {
  figures[name] = { limit, samples, unit, value };
  console.log(
    `${name}: ${value.toFixed(2)} ${unit} (target at most ${String(limit)} ${unit}, ${String(samples)} samples)`,
  );
  if (!REPORT_ONLY) {
    expect(value).toBeLessThanOrEqual(limit);
  }
}

/** How much the page's heap grew, when the webview reports its size. */
function heapGrowth(before: number | undefined, after: number | undefined): number | undefined {
  if (before === undefined) {
    return undefined;
  }
  if (after === undefined) {
    return undefined;
  }
  return after - before;
}

/** Opens a terminal of this computer with Ctrl+`, and waits until its rows show. */
async function openTerminal(): Promise<void> {
  await browser.keys([Key.Ctrl, '`']);
  await $(ROWS).waitForDisplayed({ timeout: 30_000 });
}

describe('performance of terminals (NFR-PERF-06 to 09)', () => {
  let provider: ProviderServer;
  let pid: number;

  before(async () => {
    provider = await startProviderServer();
    await waitForApp();
    pid = await processId();
    await applyLowEndProfile(pid);
    await addProfile(provider.baseUrl, { model: MODELS.fast, name: 'Fast' });
    await setPluginEnabled('terminal', true);
    await openSettings('terminal.settings');
    await $('#terminal-renderer-compatible').click();
    await leaveSettings();
    // A terminal belongs to a conversation, which its first message starts.
    await startNewConversation();
    await sendMessage('Open a conversation');
    await waitForCompleteReply(120_000);
  });

  after(async () => {
    mkdirSync(join(repository, 'target', 'perf'), { recursive: true });
    writeFileSync(
      join(repository, 'target', 'perf', 'terminal.json'),
      `${JSON.stringify({ figures, lowEnd: LOW_END, measuredAt: new Date().toISOString(), reportOnly: REPORT_ONLY }, undefined, 2)}\n`,
    );
    await deleteProviderProfiles();
    await provider.close();
  });

  it("NFR-PERF-06: Mannie adds at most 25 ms (p95) to a keystroke's echo", async () => {
    await openTerminal();
    // The page types and times on its own: WebDriver's round trips would
    // cost more than the target. xterm.js reads keys from its hidden field.
    const latencies = await browser.execute(async (rowsSelector: string) => {
      const rows = document.querySelector(rowsSelector);
      const field = document.querySelector<HTMLTextAreaElement>('[data-terminal-view] .xterm-helper-textarea');
      if (rows === null) {
        throw new Error('the terminal has no rows');
      }
      if (field === null) {
        throw new Error('the terminal has no field for keys');
      }
      const samples: number[] = [];
      for (let sample = 0; sample < 40; sample += 1) {
        const before = rows.textContent;
        const shown = new Promise<number>((resolve) => {
          const observer = new MutationObserver(() => {
            if (rows.textContent !== before) {
              observer.disconnect();
              resolve(performance.now());
            }
          });
          observer.observe(rows, { characterData: true, childList: true, subtree: true });
        });
        const start = performance.now();
        field.dispatchEvent(new KeyboardEvent('keydown', { bubbles: true, code: 'KeyX', key: 'x' }));
        field.dispatchEvent(new KeyboardEvent('keypress', { bubbles: true, charCode: 120, code: 'KeyX', key: 'x' }));
        samples.push((await shown) - start);
        await new Promise((resolve) => {
          setTimeout(resolve, 100);
        });
      }
      return samples;
    }, ROWS);
    await browser.keys([Key.Ctrl, 'c']);

    check('NFR-PERF-06 echo p95', percentile(latencies, 0.95), 25, 'ms', latencies.length);
  });

  it('NFR-PERF-07: while a visible terminal floods, input answers within 50 ms (p95) and the view settles within 300 ms', async () => {
    await browser.execute((rowsSelector: string) => {
      const rows = document.querySelector(rowsSelector);
      const box = document.querySelector<HTMLTextAreaElement>('textarea[aria-describedby="composer-hint"]');
      if (rows === null) {
        throw new Error('the terminal has no rows');
      }
      if (box === null) {
        throw new Error('the page has no message box');
      }
      new MutationObserver(() => {
        window.lastRowChange = performance.now();
      }).observe(rows, { characterData: true, childList: true, subtree: true });
      if (PerformanceObserver.supportedEntryTypes.includes('longtask')) {
        window.longestTask = 0;
        new PerformanceObserver((list) => {
          for (const entry of list.getEntries()) {
            window.longestTask = Math.max(window.longestTask ?? 0, entry.duration);
          }
        }).observe({ type: 'longtask' });
      }
      const samples: number[] = [];
      window.floodLatencies = samples;
      window.floodSamplingStopped = false;
      const sample = (): void => {
        if (window.floodSamplingStopped === true) {
          return;
        }
        const start = performance.now();
        box.value += 'x';
        box.dispatchEvent(new Event('input', { bubbles: true }));
        requestAnimationFrame(() => {
          setTimeout(() => {
            samples.push(performance.now() - start);
            setTimeout(sample, 100);
          }, 0);
        });
      };
      sample();
    }, ROWS);
    await $('[data-terminal-view]').click();
    await browser.keys(FLOOD.split(''));
    await browser.keys(Key.Enter);
    await browser.pause(FLOOD_MILLIS);
    await browser.keys([Key.Ctrl, 'c']);
    const stopped = await browser.execute(() => performance.now());
    // The view has settled once its rows stay unchanged for a second.
    await browser.waitUntil(
      async () => {
        const last = await browser.execute(() => window.lastRowChange ?? 0);
        const now = await browser.execute(() => performance.now());
        return now - last > 1_000;
      },
      { interval: 200, timeout: 30_000, timeoutMsg: 'the terminal did not settle after the flood' },
    );
    const measured = await browser.execute(() => {
      window.floodSamplingStopped = true;
      return {
        lastRowChange: window.lastRowChange ?? 0,
        latencies: window.floodLatencies ?? [],
        longestTask: window.longestTask,
      };
    });

    expect(measured.latencies.length).toBeGreaterThan(50);
    check(
      'NFR-PERF-07 input latency p95 during a flood',
      percentile(measured.latencies, 0.95),
      50,
      'ms',
      measured.latencies.length,
    );
    check('NFR-PERF-07 view settled after the flood', Math.max(0, measured.lastRowChange - stopped), 300, 'ms');
    if (measured.longestTask === undefined) {
      console.log('NFR-PERF-07 longest task: this webview reports no long tasks');
    } else {
      check('NFR-PERF-07 longest task during a flood', measured.longestTask, 50, 'ms');
    }
  });

  // The idle measurement takes two minutes and the terminals to open, longer
  // than one test may run, so its steps are tests in order.
  let quietCpu = 0;
  let heapBefore: number | undefined;

  it('measures a quiet minute with one terminal', async () => {
    heapBefore = await browser.execute(
      () => (performance as Performance & { memory?: { usedJSHeapSize: number } }).memory?.usedJSHeapSize,
    );
    const quietBefore = cpuMilliseconds(pid);
    await browser.pause(IDLE_MILLIS);
    quietCpu = cpuMilliseconds(pid) - quietBefore;
  });

  it('opens 7 more terminals', async () => {
    // The first terminal is open; the rest open as new tabs.
    for (let terminal = 2; terminal <= IDLE_TERMINALS; terminal += 1) {
      await $('[data-terminal-control]').click();
      await $('[role="menuitem"]=New terminal').click();
      // The first shell of this computer, the default.
      const [shell] = await $$('[data-slot="dropdown-menu-sub-content"] [role="menuitem"]').filter((item) =>
        item.isDisplayed(),
      );
      await shell?.click();
      await browser.waitUntil(async () => (await $$('[data-side-panel-tab]').length) >= terminal, {
        timeout: 30_000,
        timeoutMsg: `terminal ${String(terminal)} did not open`,
      });
    }
  });

  it('NFR-PERF-08 and 09: 8 idle terminals add under 0.5% of one core, and each tab at most 10 MB', async () => {
    const idleBefore = cpuMilliseconds(pid);
    await browser.pause(IDLE_MILLIS);
    const idleCpu = cpuMilliseconds(pid) - idleBefore;
    const heapAfter = await browser.execute(
      () => (performance as Performance & { memory?: { usedJSHeapSize: number } }).memory?.usedJSHeapSize,
    );

    const addedShare = ((idleCpu - quietCpu) / IDLE_MILLIS) * 100;
    check('NFR-PERF-08 CPU that 8 idle terminals add', Math.max(0, addedShare), 0.5, '% of one core');
    const growth = heapGrowth(heapBefore, heapAfter);
    if (growth === undefined) {
      console.log('NFR-PERF-09 memory per tab: this webview reports no heap size');
    } else {
      const perTab = growth / (IDLE_TERMINALS - 1) / 1_000_000;
      check('NFR-PERF-09 memory per terminal tab', Math.max(0, perTab), 10, 'MB', IDLE_TERMINALS - 1);
    }
  });
});
