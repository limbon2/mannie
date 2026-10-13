// V2 and V3: typed commands and channel streams through the generated bindings
// (ADR-0007), and the streaming load of eight runs in batches every 16 to 33 ms
// (ADR-0009, ADR-0012).
import { Key } from 'webdriverio';

import { runChannelLoad } from '../support/harness.ts';
import { waitForApp, waitForShownText } from '../support/page.ts';

describe('typed IPC (V2, V3)', () => {
  before(async () => {
    await waitForApp();
  });

  it('reads and changes the interface settings through typed commands', async () => {
    await browser.keys([Key.Ctrl, ',']);
    await waitForShownText('#interface-language', /System language/);

    await $('#interface-language').click();
    await $('[data-slot="select-item"][data-value="en"]').click();
    await waitForApp();
    await browser.keys([Key.Ctrl, ',']);

    await waitForShownText('#interface-language', 'English');
  });

  it('streams a channel in order and without gaps', async () => {
    const report = await runChannelLoad({ deltasPerSecond: 100, durationMillis: 1_000, runs: 1 });

    expect(report.sent?.deltas).toBe(report.deltas);
    expect(report.deltas).toBeGreaterThanOrEqual(95);
    expect(report.gaps).toBe(0);
    expect(report.outOfOrder).toBe(0);
  });

  it('carries 8 runs of 200 deltas per second in frame-paced batches (V3)', async () => {
    const report = await runChannelLoad({ deltasPerSecond: 200, durationMillis: 10_000, runs: 8 });
    console.log(`V3 channel load: ${JSON.stringify(report)}`);

    expect(report.sent?.deltas).toBe(report.deltas);
    expect(report.deltas).toBeGreaterThanOrEqual(15_900);
    expect(report.gaps).toBe(0);
    expect(report.outOfOrder).toBe(0);
    // The core batches every 16 ms; the webview receives a batch every 16 to 33 ms.
    expect(report.batchIntervals.p50).toBeGreaterThanOrEqual(12);
    expect(report.batchIntervals.p50).toBeLessThanOrEqual(33);
    expect(report.batchIntervals.p95).toBeLessThanOrEqual(50);
    // A delta reaches the webview well within NFR-PERF-01's 100 ms.
    expect(report.deliveryLatency.p95).toBeLessThan(100);
  });
});
