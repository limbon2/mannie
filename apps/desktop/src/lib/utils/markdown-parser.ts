import type { MarkdownRequest, MarkdownResponse } from './markdown-protocol.js';
// The window's side of the Markdown worker (ADR-0021): one request at a time,
// in order, and a time limit after which the worker is replaced and the text
// shows as plain text. The worker keeps the window responsive whatever the
// text is; the limit keeps one slow text from holding up the others.
import type { RenderedMarkdown } from './markdown.js';

/** How long one parse may take in the worker. */
export const PARSE_TIME_LIMIT_MILLIS = 3_000;

/** The part of a Web Worker that the client uses. */
export interface ParserWorker {
  onerror: ((event: ErrorEvent) => void) | null;
  onmessage: ((event: MessageEvent<MarkdownResponse>) => void) | null;
  postMessage(message: MarkdownRequest): void;
  terminate(): void;
}

/** Timers, injectable for tests. */
export interface Timers {
  clear(timer: number): void;
  start(callback: () => void, millis: number): number;
}

interface Job {
  readonly request: Extract<MarkdownRequest, { id: number }>;
  readonly resolve: (rendered: RenderedMarkdown) => void;
}

const browserTimers: Timers = {
  clear: (timer) => {
    clearTimeout(timer);
  },
  start: (callback, millis) => window.setTimeout(callback, millis),
};

/** Parses texts in a worker, one at a time, within the time limit. */
export class MarkdownWorkerClient {
  readonly #createWorker: () => ParserWorker;
  readonly #timers: Timers;
  #worker: ParserWorker | null = null;
  readonly #queue: Job[] = [];
  #current: { readonly job: Job; readonly timer: number } | null = null;
  #nextId = 1;
  #nextStream = 1;

  constructor(createWorker: () => ParserWorker, timers: Timers = browserTimers) {
    this.#createWorker = createWorker;
    this.#timers = timers;
  }

  /** A new stream, whose finished blocks the worker keeps between requests. */
  newStream(): number {
    const stream = this.#nextStream;
    this.#nextStream += 1;
    return stream;
  }

  /** Parses a whole text. */
  parse(text: string): Promise<RenderedMarkdown> {
    return this.#enqueue({ id: this.#takeId(), kind: 'parse', text });
  }

  /** Parses the latest text of a stream. */
  parseStream(stream: number, text: string): Promise<RenderedMarkdown> {
    return this.#enqueue({ id: this.#takeId(), kind: 'stream', stream, text });
  }

  /** Lets the worker forget a stream's finished blocks. */
  forget(stream: number): void {
    this.#worker?.postMessage({ kind: 'forget', stream });
  }

  #takeId(): number {
    const id = this.#nextId;
    this.#nextId += 1;
    return id;
  }

  #enqueue(request: Job['request']): Promise<RenderedMarkdown> {
    return new Promise((resolve) => {
      this.#queue.push({ request, resolve });
      this.#next();
    });
  }

  #next(): void {
    if (this.#current !== null) {
      return;
    }
    const job = this.#queue.shift();
    if (job === undefined) {
      return;
    }
    const worker = this.#ensureWorker();
    if (worker === null) {
      job.resolve({ kind: 'plain', text: job.request.text });
      this.#next();
      return;
    }
    const timer = this.#timers.start(() => {
      this.#abandon();
    }, PARSE_TIME_LIMIT_MILLIS);
    this.#current = { job, timer };
    worker.postMessage(job.request);
  }

  #ensureWorker(): ParserWorker | null {
    if (this.#worker !== null) {
      return this.#worker;
    }
    try {
      const worker = this.#createWorker();
      worker.onmessage = (event) => {
        this.#answer(event.data);
      };
      worker.onerror = () => {
        this.#abandon();
      };
      this.#worker = worker;
      return worker;
    } catch {
      return null;
    }
  }

  #answer(response: MarkdownResponse): void {
    const current = this.#current;
    if (current?.job.request.id !== response.id) {
      return;
    }
    this.#timers.clear(current.timer);
    this.#current = null;
    current.job.resolve(response.rendered);
    this.#next();
  }

  /**
   * Replaces the worker, because it took too long or failed, and shows the
   * text it was parsing as plain text.
   */
  #abandon(): void {
    const current = this.#current;
    this.#worker?.terminate();
    this.#worker = null;
    if (current === null) {
      return;
    }
    this.#timers.clear(current.timer);
    this.#current = null;
    current.job.resolve({ kind: 'plain', text: current.job.request.text });
    this.#next();
  }
}

/** The window's Markdown worker, started on the first request that needs it. */
export const markdownWorker = new MarkdownWorkerClient(
  () => new Worker(new URL('./markdown-worker.ts', import.meta.url), { type: 'module' }),
);
