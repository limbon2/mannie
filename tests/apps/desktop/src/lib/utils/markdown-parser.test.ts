// The window's side of the Markdown worker (ADR-0021): requests go one at a
// time in order, and a worker that takes too long or fails is replaced while
// its text shows as plain text.
import { describe, expect, it } from 'vitest';

import {
  MarkdownWorkerClient,
  PARSE_TIME_LIMIT_MILLIS,
  type ParserWorker,
  type Timers,
} from '../../../../../../apps/desktop/src/lib/utils/markdown-parser.js';
import type {
  MarkdownRequest,
  MarkdownResponse,
} from '../../../../../../apps/desktop/src/lib/utils/markdown-protocol.js';

/** A worker that the test answers by hand. */
class FakeWorker implements ParserWorker {
  onerror: ((event: ErrorEvent) => void) | null = null;
  onmessage: ((event: MessageEvent<MarkdownResponse>) => void) | null = null;
  readonly received: MarkdownRequest[] = [];
  terminated = false;

  /** Answers the request at `index` with a tree of one text. */
  answer(index: number, text: string): void {
    const data: MarkdownResponse = {
      id: idOf(this.received[index]),
      rendered: { kind: 'markdown', nodes: [{ kind: 'text', value: text }] },
    };
    this.onmessage?.({ data } as MessageEvent<MarkdownResponse>);
  }

  postMessage(message: MarkdownRequest): void {
    this.received.push(message);
  }

  terminate(): void {
    this.terminated = true;
  }
}

/** Timers that fire when the test says so. */
class FakeTimers implements Timers {
  readonly durations: number[] = [];
  #next = 1;
  readonly #pending = new Map<number, () => void>();

  clear(timer: number): void {
    this.#pending.delete(timer);
  }

  fireAll(): void {
    const callbacks = [...this.#pending.values()];
    this.#pending.clear();
    for (const callback of callbacks) {
      callback();
    }
  }

  start(callback: () => void, millis: number): number {
    const timer = this.#next;
    this.#next += 1;
    this.durations.push(millis);
    this.#pending.set(timer, callback);
    return timer;
  }
}

function idOf(request: MarkdownRequest | undefined): number {
  if (request === undefined) {
    throw new Error('expected a request');
  }
  if (request.kind === 'forget') {
    throw new Error('expected a parse request');
  }
  return request.id;
}

function setUp(): { client: MarkdownWorkerClient; timers: FakeTimers; workers: FakeWorker[] } {
  const workers: FakeWorker[] = [];
  const timers = new FakeTimers();
  const client = new MarkdownWorkerClient(() => {
    const worker = new FakeWorker();
    workers.push(worker);
    return worker;
  }, timers);
  return { client, timers, workers };
}

function worker(workers: readonly FakeWorker[], index: number): FakeWorker {
  const found = workers.at(index);
  if (found === undefined) {
    throw new Error(`expected worker ${String(index)}`);
  }
  return found;
}

describe('MarkdownWorkerClient', () => {
  it('sends one request at a time, in order', async () => {
    const { client, timers, workers } = setUp();

    const first = client.parse('first');
    const second = client.parse('second');
    expect(worker(workers, 0).received.map((request) => request.kind)).toEqual(['parse']);

    worker(workers, 0).answer(0, 'one');
    expect(await first).toEqual({ kind: 'markdown', nodes: [{ kind: 'text', value: 'one' }] });
    expect(worker(workers, 0).received).toHaveLength(2);
    worker(workers, 0).answer(1, 'two');

    expect(await second).toEqual({ kind: 'markdown', nodes: [{ kind: 'text', value: 'two' }] });
    expect(timers.durations).toEqual([PARSE_TIME_LIMIT_MILLIS, PARSE_TIME_LIMIT_MILLIS]);
    expect(workers).toHaveLength(1);
  });

  it('shows a text as plain text when its parse takes too long, and starts a new worker', async () => {
    const { client, timers, workers } = setUp();

    const slow = client.parse('*a'.repeat(10));
    const next = client.parse('next');
    timers.fireAll();

    expect(await slow).toEqual({ kind: 'plain', text: '*a'.repeat(10) });
    expect(worker(workers, 0).terminated).toBe(true);
    expect(worker(workers, 1).received).toHaveLength(1);
    worker(workers, 1).answer(0, 'fine');
    expect(await next).toEqual({ kind: 'markdown', nodes: [{ kind: 'text', value: 'fine' }] });
  });

  it('shows a text as plain text when the worker fails', async () => {
    const { client, workers } = setUp();

    const failed = client.parse('text');
    worker(workers, 0).onerror?.(new Event('error') as ErrorEvent);

    expect(await failed).toEqual({ kind: 'plain', text: 'text' });
    expect(worker(workers, 0).terminated).toBe(true);
  });

  it('ignores an answer that arrives after the time limit', async () => {
    const { client, timers, workers } = setUp();
    const late = client.parse('late');

    timers.fireAll();
    worker(workers, 0).answer(0, 'too late');

    expect(await late).toEqual({ kind: 'plain', text: 'late' });
  });

  it('streams with the stream it names and lets the worker forget it', () => {
    const { client, workers } = setUp();
    const stream = client.newStream();

    void client.parseStream(stream, 'partial');
    client.forget(stream);

    expect(worker(workers, 0).received).toEqual([
      { id: 1, kind: 'stream', stream, text: 'partial' },
      { kind: 'forget', stream },
    ]);
  });

  it('shows plain text when no worker can start', async () => {
    const client = new MarkdownWorkerClient(() => {
      throw new Error('workers are not available');
    }, new FakeTimers());

    expect(await client.parse('text')).toEqual({ kind: 'plain', text: 'text' });
  });
});
