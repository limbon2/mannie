// The performance targets of M1 (NFR-PERF-01 to 04, docs/product/
// goals-and-scope.md), measured in the real app. The suite enforces them on
// the reference machine (`task e2e:perf`). Elsewhere, such as in CI,
// `MANNIE_PERF_REPORT_ONLY=1` reports the figures without failing, because
// shared machines measure their neighbours as much as Mannie. Every figure is
// also written to target/perf/results.json for the validation record.
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { repository } from '../support/app.ts';
import {
  addProfile,
  COMPOSER,
  sendMessage,
  startNewConversation,
  useProfile,
  waitForCompleteReply,
  waitForComposer,
  waitForLatestPart,
} from '../support/chat.ts';
import { deleteProviderProfiles, processId, seedConversation } from '../support/harness.ts';
import { waitForApp } from '../support/page.ts';
import { MODELS, type ProviderServer, startProviderServer, TIMED_PIECES } from '../support/provider-server.ts';

const REPORT_ONLY = process.env.MANNIE_PERF_REPORT_ONLY === '1';

/** Replies that stream at the same time for NFR-PERF-02. */
const CONCURRENT_REPLIES = 8;

/** Turns of the long conversation, each with the owner's message and a reply: 10,000 parts. */
const SEEDED_TURNS = 5_000;

interface Figure {
  readonly limit: number;
  readonly samples: number;
  readonly unit: string;
  readonly value: number;
}

declare global {
  interface Window {
    /** Times from the provider sending a piece to the page showing it. */
    deltaLatencies?: number[];
    /** Times from input to the next painted frame while a reply streams. */
    inputLatencies?: number[];
    /** When the owner clicked the long conversation, and when its turns showed. */
    openTiming?: { clicked: number; shown: number };
    /** Ends the sampling of input latencies. */
    samplingStopped?: boolean;
  }
}

const figures: Record<string, Figure> = {};

function percentile(values: readonly number[], fraction: number): number {
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * fraction) - 1)] ?? Number.NaN;
}

/** Records a figure and, unless the run only reports, checks it against its target. */
function check(name: string, value: number, limit: number, samples = 1): void {
  figures[name] = { limit, samples, unit: 'ms', value };
  console.log(`${name}: ${value.toFixed(1)} ms (target at most ${String(limit)} ms, ${String(samples)} samples)`);
  if (!REPORT_ONLY) {
    expect(value).toBeLessThanOrEqual(limit);
  }
}

/** When a process started, in milliseconds since the Unix epoch, as the operating system recorded it. */
function processStartTime(pid: number): number {
  if (process.platform === 'win32') {
    const started = execFileSync(
      'powershell',
      ['-NoProfile', '-Command', `(Get-Process -Id ${String(pid)}).StartTime.ToUniversalTime().ToString('o')`],
      { encoding: 'utf8' },
    );
    return Date.parse(started.trim());
  }
  // /proc/<pid>/stat field 22 counts clock ticks since boot; /proc/stat has the boot time.
  const stat = readFileSync(`/proc/${String(pid)}/stat`, 'utf8');
  const fields = stat.slice(stat.lastIndexOf(')') + 2).split(' ');
  const ticks = Number(fields[19]);
  const bootTime = Number(/^btime (\d+)$/mu.exec(readFileSync('/proc/stat', 'utf8'))?.[1]);
  const ticksPerSecond = Number(execFileSync('getconf', ['CLK_TCK'], { encoding: 'utf8' }).trim());
  return (bootTime + ticks / ticksPerSecond) * 1_000;
}

/** How many conversations show a reply that is being written. */
async function activeReplies(): Promise<number> {
  return browser.execute(
    () => document.querySelectorAll('[data-slot="sidebar"] a[href*="conversation="] svg[role="status"]').length,
  );
}

describe('performance (NFR-PERF-01 to 04)', () => {
  let provider: ProviderServer;

  before(async () => {
    provider = await startProviderServer();
    await waitForApp();
    // The first profile is the default, which new conversations use.
    await addProfile(provider.baseUrl, { model: MODELS.slow, name: 'Slow' });
    await addProfile(provider.baseUrl, { model: MODELS.timed, name: 'Timed' });
    await addProfile(provider.baseUrl, { model: MODELS.fast, name: 'Fast' });
  });

  after(async () => {
    mkdirSync(join(repository, 'target', 'perf'), { recursive: true });
    writeFileSync(
      join(repository, 'target', 'perf', 'results.json'),
      `${JSON.stringify({ figures, measuredAt: new Date().toISOString(), reportOnly: REPORT_ONLY }, undefined, 2)}\n`,
    );
    await deleteProviderProfiles();
    await provider.close();
  });

  it('NFR-PERF-03: a cold start shows a usable window within 2 s', async () => {
    await browser.reloadSession();
    // The operating system recorded the start, so asking once the page has
    // loaded changes nothing.
    await waitForApp();
    const started = processStartTime(await processId());
    await waitForComposer();
    const usable = await browser.execute(() => Date.now());

    check('NFR-PERF-03 cold start', usable - started, 2_000);
  });

  it('NFR-PERF-01: a streamed piece appears within 100 ms (p95)', async () => {
    // A new conversation starts with the default profile; its reply stops, so
    // that the timed one can follow in the same conversation.
    await startNewConversation();
    await sendMessage('Open a conversation');
    await waitForLatestPart('text', ['streaming']);
    await $(COMPOSER).click();
    await browser.keys('Escape');
    await waitForLatestPart('text', ['stopped']);
    await useProfile('Timed');
    await browser.execute(() => {
      const latencies: number[] = [];
      const seen = new Set<string>();
      window.deltaLatencies = latencies;
      const main = document.querySelector('main');
      if (main === null) {
        throw new Error('the page has no main area');
      }
      new MutationObserver(() => {
        const shownAt = Date.now();
        const turns = main.querySelectorAll('article');
        const reply = turns.item(turns.length - 1).querySelector('[data-part-kind="text"]');
        for (const match of (reply?.textContent ?? '').matchAll(/\[t=(\d+)\]/gu)) {
          if (!seen.has(match[0])) {
            seen.add(match[0]);
            latencies.push(shownAt - Number(match[1]));
          }
        }
      }).observe(main, { characterData: true, childList: true, subtree: true });
    });

    await sendMessage('Time the pieces');
    await waitForCompleteReply(60_000);

    const latencies = (await browser.execute(() => window.deltaLatencies)) ?? [];
    expect(latencies.length).toBeGreaterThanOrEqual(TIMED_PIECES * 0.9);
    check('NFR-PERF-01 piece latency p95', percentile(latencies, 0.95), 100, latencies.length);
  });

  it('NFR-PERF-02: with 8 replies streaming, the window answers input within 50 ms (p95)', async () => {
    for (let reply = 1; reply <= CONCURRENT_REPLIES; reply += 1) {
      await startNewConversation();
      await sendMessage(`Stream ${String(reply)}`);
      await waitForLatestPart('text', ['streaming']);
    }
    expect(await activeReplies()).toBe(CONCURRENT_REPLIES);

    // Input to the next painted frame, from the page itself: WebDriver's own
    // round trips would cost more than the target.
    const latencies = await browser.execute(async () => {
      const box = document.querySelector<HTMLTextAreaElement>('textarea[aria-describedby="composer-hint"]');
      if (box === null) {
        throw new Error('the page has no message box');
      }
      const samples: number[] = [];
      for (let sample = 0; sample < 60; sample += 1) {
        const start = performance.now();
        box.value += 'x';
        box.dispatchEvent(new Event('input', { bubbles: true }));
        await new Promise((resolve) => {
          requestAnimationFrame(() => {
            setTimeout(resolve, 0);
          });
        });
        samples.push(performance.now() - start);
        await new Promise((resolve) => {
          setTimeout(resolve, 50);
        });
      }
      return samples;
    });

    check('NFR-PERF-02 input latency p95', percentile(latencies, 0.95), 50, latencies.length);
    await browser.waitUntil(async () => (await activeReplies()) === 0, {
      timeout: 60_000,
      timeoutMsg: 'the replies did not end',
    });
  });

  it('a reply of 32 KB at 200 pieces a second leaves the view answering input within 50 ms (p95)', async () => {
    await startNewConversation();
    await sendMessage('Open a conversation');
    await waitForLatestPart('text', ['streaming']);
    await $(COMPOSER).click();
    await browser.keys('Escape');
    await waitForLatestPart('text', ['stopped']);
    await useProfile('Fast');
    // The page samples on its own while the reply streams, longer than one
    // WebDriver script may run: input every 250 ms, timed to the next frame.
    await browser.execute(() => {
      const box = document.querySelector<HTMLTextAreaElement>('textarea[aria-describedby="composer-hint"]');
      if (box === null) {
        throw new Error('the page has no message box');
      }
      const samples: number[] = [];
      window.inputLatencies = samples;
      window.samplingStopped = false;
      const sample = (): void => {
        if (window.samplingStopped === true) {
          return;
        }
        const start = performance.now();
        box.value += 'x';
        box.dispatchEvent(new Event('input', { bubbles: true }));
        requestAnimationFrame(() => {
          setTimeout(() => {
            samples.push(performance.now() - start);
            setTimeout(sample, 250);
          }, 0);
        });
      };
      sample();
    });

    await sendMessage('Write a lot');
    await waitForLatestPart('text', ['streaming']);
    await waitForCompleteReply(120_000);

    const latencies = await browser.execute(() => {
      window.samplingStopped = true;
      return window.inputLatencies ?? [];
    });
    expect(latencies.length).toBeGreaterThan(100);
    check('32 KB reply, input latency p95', percentile(latencies, 0.95), 50, latencies.length);
  });

  it('NFR-PERF-04: a conversation with 10,000 parts opens within 1 s', async () => {
    const id = await seedConversation(SEEDED_TURNS);
    const link = await $(`[data-slot="sidebar"] a[href*="${id}"]`);
    await link.waitForExist();
    await browser.execute(() => {
      const timing = { clicked: 0, shown: 0 };
      window.openTiming = timing;
      document.addEventListener(
        'click',
        () => {
          timing.clicked = performance.now();
        },
        { capture: true, once: true },
      );
      const main = document.querySelector('main');
      if (main === null) {
        throw new Error('the page has no main area');
      }
      const observer = new MutationObserver(() => {
        const clicked = timing.clicked > 0;
        const hasTurns = main.querySelectorAll('article').length > 0;
        if (clicked && hasTurns) {
          timing.shown = performance.now();
          observer.disconnect();
        }
      });
      observer.observe(main, { childList: true, subtree: true });
    });

    await link.click();

    await browser.waitUntil(async () => ((await browser.execute(() => window.openTiming?.shown)) ?? 0) > 0, {
      timeout: 20_000,
      timeoutMsg: 'the long conversation did not open',
    });
    const timing = await browser.execute(() => window.openTiming);
    check('NFR-PERF-04 open a conversation of 10,000 parts', (timing?.shown ?? 0) - (timing?.clicked ?? 0), 1_000);
  });
});
