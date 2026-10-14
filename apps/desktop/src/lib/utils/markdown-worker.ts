// The Markdown worker (ADR-0021): parses model output off the window's main
// thread, so that a text whose parse takes long never freezes the window. The
// window abandons a parse that takes too long and shows the text as plain text.
import type { MarkdownRequest, MarkdownResponse } from './markdown-protocol.js';
import { parseMarkdown, StreamingMarkdown } from './markdown.js';

/** The part of a dedicated worker's global scope that this worker uses. */
interface WorkerScope {
  addEventListener(type: 'message', listener: (event: MessageEvent<MarkdownRequest>) => void): void;
  postMessage(message: MarkdownResponse): void;
}

const scope = globalThis as unknown as WorkerScope;

/** The finished blocks of each streaming reply. */
const streams = new Map<number, StreamingMarkdown>();

scope.addEventListener('message', (event) => {
  const request = event.data;
  switch (request.kind) {
    case 'forget': {
      streams.delete(request.stream);
      return;
    }
    case 'parse': {
      scope.postMessage({ id: request.id, rendered: parseMarkdown(request.text) });
      return;
    }
    case 'stream': {
      let stream = streams.get(request.stream);
      if (stream === undefined) {
        stream = new StreamingMarkdown();
        streams.set(request.stream, stream);
      }
      scope.postMessage({ id: request.id, rendered: stream.update(request.text) });
    }
  }
});
