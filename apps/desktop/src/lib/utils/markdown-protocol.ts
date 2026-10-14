// The messages between the window and the Markdown worker (ADR-0021).
import type { RenderedMarkdown } from './markdown.js';

/** What the window asks the worker. */
export type MarkdownRequest =
  /** Parses a whole text. */
  | { readonly id: number; readonly kind: 'parse'; readonly text: string }
  /** Parses the latest text of a streaming reply, reusing its finished blocks. */
  | { readonly id: number; readonly kind: 'stream'; readonly stream: number; readonly text: string }
  /** Forgets the finished blocks of a stream. No answer follows. */
  | { readonly kind: 'forget'; readonly stream: number };

/** The worker's answer to a parse or stream request. */
export interface MarkdownResponse {
  readonly id: number;
  readonly rendered: RenderedMarkdown;
}
