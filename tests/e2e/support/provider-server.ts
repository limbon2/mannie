// A model provider for the end-to-end tests (docs/features/chat/providers.md):
// a local HTTP server that speaks the Responses API and Chat Completions,
// lists its models, and answers each request after the scenario that the
// request's model names. It records every request, so that a test can check
// what the app sent, and it answers the probes for known servers with 404, so
// the app treats it as a server that it does not recognize.
import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';

/** The scenarios, by model name. */
export const MODELS = {
  /** Answers in several pieces with text that names the owner's message. */
  echo: 'e2e-echo',
  /** Answers with 32 KB of Markdown in pieces of about four bytes, 200 pieces a second. */
  fast: 'e2e-fast',
  /** Answers with the hostile Markdown that the test gave the server. */
  hostile: 'e2e-hostile',
  /** Sends a little text, then waits until the app stops the reply or the server closes. */
  hang: 'e2e-hang',
  /** Answers with long replies and summarizes on request, for small context windows. */
  long: 'e2e-long',
  /** Answers with numbered words, one every 25 ms: `w1 w2 … w200`. */
  paced: 'e2e-paced',
  /** Refuses the first request with 429 and `Retry-After`, then answers. */
  rateLimited: 'e2e-rate-limited',
  /** Reasons before it answers. */
  reasoning: 'e2e-reasoning',
  /** Answers with numbered words for about 20 seconds, for replies that run at the same time. */
  slow: 'e2e-slow',
  /** Answers with pieces that carry the time they were sent: `[t=<ms since the epoch>]`. */
  timed: 'e2e-timed',
  /** Refuses every request as if the credential were wrong, and repeats the credential as some servers do. */
  unauthorized: 'e2e-unauthorized',
} as const;

/** How many words the paced scenario sends. */
export const PACED_WORDS = 200;

/** How long the paced scenario waits between words. */
const PACED_INTERVAL_MILLIS = 25;

/** How many words the slow scenario sends, one every 50 ms. */
const SLOW_WORDS = 400;

/** How many pieces the timed scenario sends, one every 20 ms. */
export const TIMED_PIECES = 250;

/** How long the rate-limited scenario asks the app to wait, in seconds. */
export const RETRY_AFTER_SECONDS = 2;

/** The context window that the model list reports for every model. */
export const LISTED_CONTEXT_WINDOW = 65_536;

/** The text that a summary of the long scenario starts with. */
export const SUMMARY_PREFIX = 'Checkpoint written by the test provider';

/** The size of the fast scenario's reply. */
export const FAST_REPLY_BYTES = 32 * 1_024;

/** The fast scenario's reply: Markdown with emphasis, code and lists, in pieces of up to four characters. */
function fastPieces(): string[] {
  const paragraphs: string[] = [];
  let length = 0;
  for (let index = 1; length < FAST_REPLY_BYTES; index += 1) {
    const paragraph = `Paragraph ${String(index)} with **bold text**, \`code\` and a list:\n\n- first item\n- second item\n\n${'Plain words follow here. '.repeat(8)}\n\n`;
    paragraphs.push(paragraph);
    length += paragraph.length;
  }
  return (
    paragraphs
      .join('')
      .slice(0, FAST_REPLY_BYTES)
      .match(/[\s\S]{1,4}/gu) ?? []
  );
}

/** The text of the paced scenario's reply. */
export function pacedText(): string {
  return Array.from({ length: PACED_WORDS }, (_unused, index) => `w${String(index + 1)}`).join(' ');
}

/** A request that reached the server. */
export interface RecordedRequest {
  /** The Authorization header, if the request had one. */
  readonly authorization: string | null;
  /** The JSON body, if the request had one. */
  readonly body: Record<string, unknown> | null;
  readonly method: string;
  readonly path: string;
  /** When the request arrived, in milliseconds since the Unix epoch. */
  readonly receivedAt: number;
}

/** The running server. */
export interface ProviderServer {
  /** The base URL for a provider profile. */
  readonly baseUrl: string;
  /** Ends every open reply and stops the server. */
  close(): Promise<void>;
  /** The requests so far, oldest first. */
  requests(): readonly RecordedRequest[];
  /** The text that the hostile scenario answers with. */
  setHostileText(text: string): void;
}

type Interface = 'chat_completions' | 'responses';

/** Writes the events of one reply in the wire format of an interface. */
class ReplyWriter {
  readonly #interface: Interface;
  readonly #response: ServerResponse;
  #reasoningId = 0;

  constructor(response: ServerResponse, api: Interface) {
    this.#response = response;
    this.#interface = api;
    response.writeHead(200, { 'cache-control': 'no-cache', 'content-type': 'text/event-stream' });
    if (api === 'responses') {
      this.#event({ response: { id: 'resp_e2e', status: 'in_progress' }, type: 'response.created' });
    }
  }

  /** Whether the app closed the connection, for example because the owner stopped the reply. */
  get closed(): boolean {
    return this.#response.destroyed || this.#response.writableEnded;
  }

  /** Ends the reply with the usage figures. */
  finish(usage: { input: number; output: number }): void {
    if (this.#interface === 'responses') {
      this.#event({
        response: {
          id: 'resp_e2e',
          status: 'completed',
          usage: { input_tokens: usage.input, output_tokens: usage.output },
        },
        type: 'response.completed',
      });
    } else {
      this.#chunk({ choices: [{ delta: {}, finish_reason: 'stop', index: 0 }] });
      this.#chunk({ choices: [], usage: { completion_tokens: usage.output, prompt_tokens: usage.input } });
      this.#response.write('data: [DONE]\n\n');
    }
    this.#response.end();
  }

  /** Sends reasoning: a summary on the Responses API, `reasoning_content` on Chat Completions. */
  reasoning(text: string): void {
    if (this.#interface === 'responses') {
      this.#reasoningId += 1;
      const itemId = `rs_e2e_${String(this.#reasoningId)}`;
      this.#event({ delta: text, item_id: itemId, summary_index: 0, type: 'response.reasoning_summary_text.delta' });
      this.#event({
        item: {
          encrypted_content: `encrypted-${itemId}`,
          id: itemId,
          summary: [{ text, type: 'summary_text' }],
          type: 'reasoning',
        },
        type: 'response.output_item.done',
      });
    } else {
      this.#chunk({ choices: [{ delta: { reasoning_content: text }, finish_reason: null, index: 0 }] });
    }
  }

  /** Sends a piece of the answer. */
  text(delta: string): void {
    if (this.#interface === 'responses') {
      this.#event({ delta, type: 'response.output_text.delta' });
    } else {
      this.#chunk({ choices: [{ delta: { content: delta }, finish_reason: null, index: 0 }] });
    }
  }

  #chunk(chunk: Record<string, unknown>): void {
    this.#response.write(
      `data: ${JSON.stringify({ id: 'chatcmpl-e2e', object: 'chat.completion.chunk', ...chunk })}\n\n`,
    );
  }

  #event(event: { type: string } & Record<string, unknown>): void {
    this.#response.write(`event: ${event.type}\ndata: ${JSON.stringify(event)}\n\n`);
  }
}

/** The text of the owner's latest message in a request body of either interface. */
function latestOwnerText(body: Record<string, unknown>): string {
  const entries = (body.messages ?? body.input) as { content?: unknown; role?: string }[] | undefined;
  const latest = entries?.findLast((entry) => entry.role === 'user');
  const content = latest?.content;
  if (typeof content === 'string') {
    return content;
  }
  if (Array.isArray(content)) {
    return content.map((part: { text?: string }) => part.text ?? '').join('');
  }
  return '';
}

/** The interface that a path belongs to, if any. */
function interfaceOf(path: string): Interface | null {
  if (path === '/v1/responses') {
    return 'responses';
  }
  if (path === '/v1/chat/completions') {
    return 'chat_completions';
  }
  return null;
}

/** Whether a request asks for a checkpoint: the compaction call ends its only message with this request. */
export function isSummaryRequest(body: Record<string, unknown> | null): boolean {
  if (body === null) {
    return false;
  }
  return latestOwnerText(body).endsWith('Write the checkpoint now.');
}

/** A rough token count, as servers report it: about four bytes a token. */
function tokens(value: unknown): number {
  return Math.ceil(Buffer.byteLength(JSON.stringify(value)) / 4);
}

async function readBody(request: IncomingMessage): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of request) {
    chunks.push(chunk as Buffer);
  }
  return Buffer.concat(chunks).toString('utf8');
}

async function pause(millis: number): Promise<void> {
  await new Promise((resolve) => {
    setTimeout(resolve, millis);
  });
}

function refuse(response: ServerResponse, status: number, error: Record<string, unknown>, headers = {}): void {
  response.writeHead(status, { 'content-type': 'application/json', ...headers });
  response.end(JSON.stringify({ error }));
}

/** Starts the server on a free port of the loopback interface. */
export async function startProviderServer(): Promise<ProviderServer> {
  const recorded: RecordedRequest[] = [];
  const attemptsByModel = new Map<string, number>();
  const longReplies = { count: 0 };
  let hostileText = '';

  async function answer(writer: ReplyWriter, body: Record<string, unknown>): Promise<void> {
    const model = String(body.model);
    const input = tokens(body.messages ?? body.input);
    switch (model) {
      case MODELS.hang: {
        writer.text('Started.');
        // The connection stays open until the app or the server closes it.
        return;
      }
      case MODELS.fast: {
        // Ten pieces every 50 ms: 200 a second, as a fast model streams.
        const pieces = fastPieces();
        for (let start = 0; start < pieces.length; start += 10) {
          if (writer.closed) {
            return;
          }
          for (const piece of pieces.slice(start, start + 10)) {
            writer.text(piece);
          }
          await pause(50);
        }
        writer.finish({ input, output: pieces.length });
        return;
      }
      case MODELS.hostile: {
        writer.text(hostileText);
        writer.finish({ input, output: tokens(hostileText) });
        return;
      }
      case MODELS.long: {
        if (isSummaryRequest(body)) {
          const summary = `${SUMMARY_PREFIX}: the owner and the assistant exchanged numbered long replies.`;
          writer.text(summary);
          writer.finish({ input, output: tokens(summary) });
          return;
        }
        longReplies.count += 1;
        const reply = `Long reply ${String(longReplies.count)}. ${'Filler text for the context budget. '.repeat(60)}`;
        writer.text(reply);
        writer.finish({ input, output: tokens(reply) });
        return;
      }
      case MODELS.paced: {
        for (let word = 1; word <= PACED_WORDS; word += 1) {
          if (writer.closed) {
            return;
          }
          writer.text(word === 1 ? 'w1' : ` w${String(word)}`);
          await pause(PACED_INTERVAL_MILLIS);
        }
        writer.finish({ input, output: PACED_WORDS });
        return;
      }
      case MODELS.slow: {
        for (let word = 1; word <= SLOW_WORDS; word += 1) {
          if (writer.closed) {
            return;
          }
          writer.text(` s${String(word)}`);
          await pause(50);
        }
        writer.finish({ input, output: SLOW_WORDS });
        return;
      }
      case MODELS.timed: {
        for (let piece = 0; piece < TIMED_PIECES; piece += 1) {
          if (writer.closed) {
            return;
          }
          writer.text(`[t=${String(Date.now())}] `);
          await pause(20);
        }
        writer.finish({ input, output: TIMED_PIECES * 8 });
        return;
      }
      case MODELS.reasoning: {
        writer.reasoning('Thinking about the question.');
        writer.text('The answer after reasoning.');
        writer.finish({ input, output: 12 });
        return;
      }
      default: {
        const reply = `Reply to: ${latestOwnerText(body)}`;
        for (const piece of reply.match(/.{1,8}/gsu) ?? []) {
          writer.text(piece);
          await pause(10);
        }
        writer.finish({ input, output: tokens(reply) });
      }
    }
  }

  async function handle(request: IncomingMessage, response: ServerResponse): Promise<void> {
    const path = new URL(request.url ?? '/', 'http://localhost').pathname;
    const text = await readBody(request);
    const body = text === '' ? null : (JSON.parse(text) as Record<string, unknown>);
    recorded.push({
      authorization: request.headers.authorization ?? null,
      body,
      method: request.method ?? '',
      path,
      receivedAt: Date.now(),
    });
    const isModelList = request.method === 'GET' && path === '/v1/models';
    if (isModelList) {
      const data = Object.values(MODELS).map((id) => ({
        context_length: LISTED_CONTEXT_WINDOW,
        id,
        object: 'model',
        owned_by: 'mannie-e2e',
      }));
      response.writeHead(200, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ data, object: 'list' }));
      return;
    }
    const api = interfaceOf(path);
    const isCall = request.method === 'POST' && api !== null && body !== null;
    if (!isCall) {
      // Includes the probes for Ollama, LM Studio, SGLang and vLLM.
      response.writeHead(404, { 'content-type': 'application/json' });
      response.end('{"error":"not found"}');
      return;
    }
    const model = String(body.model);
    const attempt = (attemptsByModel.get(model) ?? 0) + 1;
    attemptsByModel.set(model, attempt);
    if (model === MODELS.unauthorized) {
      const credential = request.headers.authorization?.replace(/^Bearer /u, '') ?? '';
      refuse(response, 401, {
        code: 'invalid_api_key',
        message: `Incorrect API key provided: ${credential}.`,
        type: 'invalid_request_error',
      });
      return;
    }
    const isRateLimited = model === MODELS.rateLimited && attempt === 1;
    if (isRateLimited) {
      refuse(
        response,
        429,
        { code: 'rate_limit_exceeded', message: 'Rate limit reached.', type: 'requests' },
        { 'retry-after': String(RETRY_AFTER_SECONDS) },
      );
      return;
    }
    await answer(new ReplyWriter(response, api), body);
  }

  const server = createServer((request, response) => {
    handle(request, response).catch((error: unknown) => {
      if (!response.headersSent) {
        response.writeHead(500, { 'content-type': 'text/plain' });
      }
      response.end(String(error));
    });
  });
  await new Promise<void>((resolve) => {
    server.listen(0, '127.0.0.1', resolve);
  });
  const address = server.address();
  const hasNoPort = address === null || typeof address === 'string';
  if (hasNoPort) {
    throw new Error('the provider server has no port');
  }
  return {
    baseUrl: `http://127.0.0.1:${String(address.port)}/v1`,
    close: async () => {
      server.closeAllConnections();
      await new Promise<void>((resolve) => {
        server.close(() => {
          resolve();
        });
      });
    },
    requests: () => [...recorded],
    setHostileText: (text) => {
      hostileText = text;
    },
  };
}
