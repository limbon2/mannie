// The validation harness (V1, V2, V3, V11). Only validation builds load it
// (src/routes/+layout.ts); the end-to-end tests drive it through
// `window.mannieValidation`. A release never contains it.
import { Channel } from '@tauri-apps/api/core';

import type { ChannelBatch, ChannelLoad, ChannelLoadSummary, ValidationDelta } from '#lib/generated/validation.js';
import { commands } from '#lib/generated/validation.js';

/** A Content Security Policy violation the webview reported. */
export interface CspViolation {
  readonly blockedUri: string;
  readonly directive: string;
  readonly sample: string;
  /** Where the code that caused it is: file, line and column. */
  readonly source: string;
}

/** Which ways of running script the page allowed. Every one should be `false`. */
export interface ScriptProbe {
  readonly evalRan: boolean;
  readonly functionConstructorRan: boolean;
  readonly inlineHandlerRan: boolean;
  readonly inlineScriptRan: boolean;
}

/** Percentiles of a measurement, in milliseconds. */
export interface Distribution {
  readonly max: number;
  readonly p50: number;
  readonly p95: number;
}

/** What the webview received for a channel load. */
export interface ChannelLoadReport {
  /** Batches received. */
  readonly batches: number;
  /** Time between the arrival of consecutive batches. */
  readonly batchIntervals: Distribution;
  /** Time from the core creating a delta to the webview receiving it. */
  readonly deliveryLatency: Distribution;
  /** Deltas received. */
  readonly deltas: number;
  /** Batches whose first sequence number did not follow the previous batch. */
  readonly gaps: number;
  /** Deltas of a run that arrived out of order. */
  readonly outOfOrder: number;
  /** What the core says it sent. */
  readonly sent: ChannelLoadSummary | null;
}

/** The harness's interface for the end-to-end tests. */
export interface ValidationHarness {
  callUngrantedCommand(): Promise<string>;
  cspViolations(): readonly CspViolation[];
  insertHostileHtml(payloads: readonly string[]): Promise<boolean>;
  probeScripts(): Promise<ScriptProbe>;
  runChannelLoad(load: ChannelLoad): Promise<ChannelLoadReport>;
}

declare global {
  interface Window {
    /** Set by any hostile payload that manages to run. */
    compromised?: boolean;
    mannieValidation?: ValidationHarness;
  }
}

/** How long the harness waits for batches that arrive after the command's response. */
const LATE_BATCH_TIMEOUT_MILLIS = 5_000;

const violations: CspViolation[] = [];

document.addEventListener('securitypolicyviolation', (event) => {
  violations.push({
    blockedUri: event.blockedURI,
    directive: event.effectiveDirective,
    sample: event.sample,
    source: `${event.sourceFile}:${String(event.lineNumber)}:${String(event.columnNumber)}`,
  });
});

/** Waits for the given time, so that the webview can run what was queued. */
async function pause(millis: number): Promise<void> {
  await new Promise<void>((resolve) => {
    setTimeout(resolve, millis);
  });
}

/** Resolves once `done` returns true, or after `timeoutMillis`. */
async function waitUntil(done: () => boolean, timeoutMillis: number): Promise<void> {
  const deadline = performance.now() + timeoutMillis;
  while (!done()) {
    if (performance.now() > deadline) {
      return;
    }
    await new Promise<void>((resolve) => {
      setTimeout(resolve, 10);
    });
  }
}

function distribution(values: readonly number[]): Distribution {
  const sorted = [...values].sort((left, right) => left - right);
  const at = (fraction: number): number => sorted[Math.round((sorted.length - 1) * fraction)] ?? 0;
  return { max: sorted.at(-1) ?? 0, p50: at(0.5), p95: at(0.95) };
}

async function runChannelLoad(load: ChannelLoad): Promise<ChannelLoadReport> {
  const arrivals: number[] = [];
  const latencies: number[] = [];
  const lastIndexOfRun = new Map<number, number>();
  let deltas = 0;
  let gaps = 0;
  let outOfOrder = 0;
  let expectedSequence = 0;

  const channel = new Channel<ChannelBatch<ValidationDelta>>((batch) => {
    arrivals.push(performance.now());
    const receivedAt = Date.now();
    if (batch.firstSequence !== expectedSequence) {
      gaps += 1;
    }
    expectedSequence = batch.firstSequence + batch.items.length;
    for (const delta of batch.items) {
      deltas += 1;
      if (delta.createdAt !== null) {
        latencies.push(receivedAt - delta.createdAt);
      }
      const previous = lastIndexOfRun.get(delta.run) ?? -1;
      if (delta.index <= previous) {
        outOfOrder += 1;
      }
      lastIndexOfRun.set(delta.run, delta.index);
    }
  });

  const result = await commands.streamValidationEvents(load, channel);
  const sent = result.status === 'ok' ? result.data : null;
  // Large batches travel separately from the command's response, so the last
  // ones can arrive after it.
  await waitUntil(() => deltas >= (sent?.deltas ?? 0), LATE_BATCH_TIMEOUT_MILLIS);
  const intervals = arrivals.slice(1).map((arrival, index) => arrival - (arrivals[index] ?? arrival));
  return {
    batchIntervals: distribution(intervals),
    batches: arrivals.length,
    deliveryLatency: distribution(latencies),
    deltas,
    gaps,
    outOfOrder,
    sent,
  };
}

/**
 * Leaves the task in which WebDriver called the harness. While WebDriver's
 * script runs, the browser's developer tools allow eval in the page, so a
 * probe has to run in a later task, as the app's own code does.
 */
async function leaveWebDriverTask(): Promise<void> {
  await pause(0);
}

/** Whether `attempt` ran and returned true. Under the policy each attempt throws. */
function ran(attempt: () => unknown): boolean {
  try {
    return attempt() === true;
  } catch {
    return false;
  }
}

/** Tries every way of running script that the policy forbids. */
async function probeScripts(): Promise<ScriptProbe> {
  await leaveWebDriverTask();
  window.compromised = false;
  // The policy must refuse eval and the Function constructor (ADR-0004); the
  // probe checks that it does. The indirect eval keeps the bundler from
  // deoptimizing the module.
  const evalRan = ran(() => globalThis.eval('true'));
  const functionConstructorRan = ran(() => {
    // eslint-disable-next-line @typescript-eslint/no-implied-eval -- the probe exists to show that the policy blocks this
    const created = new Function('return true') as () => unknown;
    return created();
  });
  const script = document.createElement('script');
  script.textContent = 'window.compromised = true;';
  document.body.append(script);
  script.remove();
  const inlineScriptRan = window.compromised;
  window.compromised = false;
  const button = document.createElement('button');
  button.setAttribute('onclick', 'window.compromised = true;');
  document.body.append(button);
  button.click();
  button.remove();
  await pause(50);
  return { evalRan, functionConstructorRan, inlineHandlerRan: window.compromised, inlineScriptRan };
}

/**
 * Inserts each hostile payload into the page, clicks what it offers to click,
 * and reports whether any payload ran.
 */
async function insertHostileHtml(payloads: readonly string[]): Promise<boolean> {
  await leaveWebDriverTask();
  window.compromised = false;
  const host = document.createElement('div');
  document.body.append(host);
  for (const payload of payloads) {
    // Hostile on purpose: the policy must keep every payload inert.
    host.innerHTML = payload;
    for (const element of host.querySelectorAll<HTMLElement>('#hostile-link, #hostile-click')) {
      element.click();
    }
    await pause(50);
  }
  host.remove();
  return window.compromised;
}

async function callUngrantedCommand(): Promise<string> {
  try {
    await commands.validationUngrantedCommand();
    return 'allowed';
  } catch (error) {
    return String(error);
  }
}

window.mannieValidation = {
  callUngrantedCommand,
  cspViolations: () => [...violations],
  insertHostileHtml,
  probeScripts,
  runChannelLoad,
};
