// Model output as Markdown (ADR-0021): micromark with the GFM extensions
// parses the text into an mdast tree, and this module turns the tree into
// Mannie's own render tree, which has only the node kinds that Mannie's
// components render. Raw HTML, footnotes, definitions and node kinds this
// module does not know become text, so nothing in the text becomes markup
// that the renderer did not choose.
import type { Nodes, Root, RootContent } from 'mdast';
import { fromMarkdown } from 'mdast-util-from-markdown';
import { gfmFromMarkdown } from 'mdast-util-gfm';
import { gfm } from 'micromark-extension-gfm';

/** Text longer than this, in UTF-8 bytes, renders as plain text. */
export const MAX_MARKDOWN_BYTES = 200 * 1024;

/** Nesting deeper than this renders as plain text. */
export const MAX_NESTING = 32;

/** How a table column is aligned. */
export type Alignment = 'center' | 'left' | 'right' | null;

/** A node of one kind that only holds other nodes. */
export interface ParentOf<Kind extends string> {
  readonly children: readonly RenderNode[];
  readonly kind: Kind;
}

/** A node that has inline or block children, one type per kind, so that a kind narrows exactly. */
export type ParentNode =
  ParentOf<'blockquote'> | ParentOf<'delete'> | ParentOf<'emphasis'> | ParentOf<'paragraph'> | ParentOf<'strong'>;

/** One item of a list. */
export interface ListItem {
  /** For a task list item, whether it is done. */
  readonly checked: boolean | null;
  readonly children: readonly RenderNode[];
}

/** One cell of a table: its inline content. */
export type TableCell = readonly RenderNode[];

/** A node of Mannie's render tree. */
export type RenderNode =
  | ParentNode
  | {
      readonly align: readonly Alignment[];
      readonly body: readonly (readonly TableCell[])[];
      readonly head: readonly TableCell[];
      readonly kind: 'table';
    }
  | { readonly children: readonly RenderNode[]; readonly kind: 'heading'; readonly level: 1 | 2 | 3 | 4 | 5 | 6 }
  | { readonly children: readonly RenderNode[]; readonly kind: 'link'; readonly url: string }
  | { readonly description: string; readonly kind: 'image'; readonly url: string }
  | {
      readonly items: readonly ListItem[];
      readonly kind: 'list';
      readonly ordered: boolean;
      readonly start: number | null;
    }
  | { readonly kind: 'break' }
  | { readonly kind: 'code'; readonly language: string | null; readonly value: string }
  | { readonly kind: 'inline_code'; readonly value: string }
  | { readonly kind: 'text'; readonly value: string }
  | { readonly kind: 'thematic_break' };

/** A parsed text: the render tree, or the text itself when it is too large or too deep. */
export type RenderedMarkdown =
  | { readonly kind: 'markdown'; readonly nodes: readonly RenderNode[] }
  | { readonly kind: 'plain'; readonly text: string };

/** The tree is nested deeper than MAX_NESTING. */
class TooDeep extends Error {}

interface Context {
  /** The URLs of the link reference definitions, by their normalized label. */
  readonly definitions: ReadonlyMap<string, string>;
  readonly depth: number;
  /** Inside a link, an image renders as its description, so no button nests in another. */
  readonly inLink: boolean;
  readonly source: string;
}

const encoder = new TextEncoder();

/** Whether a text is too large to parse. */
function exceedsSize(text: string): boolean {
  // A UTF-16 code unit takes at most three UTF-8 bytes, so short texts need no count.
  const isShort = text.length * 3 <= MAX_MARKDOWN_BYTES;
  if (isShort) {
    return false;
  }
  // Counting bytes is only needed when the length alone does not decide.
  const exceeds = text.length > MAX_MARKDOWN_BYTES || encoder.encode(text).length > MAX_MARKDOWN_BYTES;
  return exceeds;
}

/**
 * The bounds within which a parse is quick enough for the window's main
 * thread. micromark's time grows faster than linearly with the number of
 * emphasis delimiters, brackets and table rows in one block, and its tree
 * builder recurses once per nesting level, so texts beyond these bounds are
 * parsed in the Markdown worker, which gives up after a time limit.
 */
const CHEAP_LIMITS = {
  /** `[`, which opens links, images and footnote references. */
  brackets: 1_000,
  /** `*`, `_` and `~`, which open and close emphasis and strikethrough. */
  delimiters: 1_000,
  /** Leading spaces of a line, which nest lists by indentation. */
  indentation: 128,
  /** UTF-16 code units. */
  length: 32 * 1024,
  /** Block quote and list markers at the start of one line. */
  markers: MAX_NESTING,
  /** Lines with `|`, which can be table rows. */
  pipeLines: 150,
} as const;

const BULLETS = new Set(['*', '+', '-']);
const DELIMITERS = new Set(['*', '_', '~']);
const ORDERED_DELIMITERS = new Set([')', '.']);
const MAX_ORDERED_DIGITS = 9;

const DIGITS = new Set(['0', '1', '2', '3', '4', '5', '6', '7', '8', '9']);

/** The index after an ordered list marker such as `1.` at `index`, or -1 when there is none. */
function orderedMarkerEnd(line: string, index: number): number {
  let end = index;
  const last = Math.min(line.length, index + MAX_ORDERED_DIGITS);
  while (end < last) {
    if (!DIGITS.has(line.charAt(end))) {
      break;
    }
    end += 1;
  }
  const hasDigits = end > index;
  const hasDelimiter = ORDERED_DELIMITERS.has(line.charAt(end));
  return hasDigits && hasDelimiter ? end + 1 : -1;
}

/** The container markers at the start of a line: `>`, `-`, `*`, `+` and `1.` or `1)`. */
function leadingMarkers(line: string): number {
  let markers = 0;
  let index = 0;
  while (markers <= CHEAP_LIMITS.markers) {
    while (line.charAt(index) === ' ') {
      index += 1;
    }
    const char = line.charAt(index);
    const next = line.charAt(index + 1);
    const isBullet = BULLETS.has(char);
    const endsMarker = next === ' ' || next === '';
    const ordered = orderedMarkerEnd(line, index);
    if (char === '>') {
      index += 1;
    } else if (isBullet && endsMarker) {
      index += 1;
    } else if (ordered === -1) {
      return markers;
    } else {
      index = ordered;
    }
    markers += 1;
  }
  return markers;
}

/** Whether one line nests too deeply for the main thread. */
function nestsTooDeeply(line: string): boolean {
  const indentation = line.length - line.trimStart().length;
  const isDeeplyIndented = indentation > CHEAP_LIMITS.indentation;
  const hasManyMarkers = leadingMarkers(line) > CHEAP_LIMITS.markers;
  return isDeeplyIndented || hasManyMarkers;
}

/** Whether the main thread can parse a text without making the window wait. */
export function isCheapToParse(text: string): boolean {
  if (text.length > CHEAP_LIMITS.length) {
    return false;
  }
  let delimiters = 0;
  let brackets = 0;
  let pipeLines = 0;
  for (const line of text.split('\n')) {
    if (nestsTooDeeply(line)) {
      return false;
    }
    if (line.includes('|')) {
      pipeLines += 1;
    }
    for (const char of line) {
      if (DELIMITERS.has(char)) {
        delimiters += 1;
      } else if (char === '[') {
        brackets += 1;
      }
    }
  }
  const fewDelimiters = delimiters <= CHEAP_LIMITS.delimiters;
  const fewBrackets = brackets <= CHEAP_LIMITS.brackets;
  const fewTableRows = pipeLines <= CHEAP_LIMITS.pipeLines;
  return fewDelimiters && fewBrackets && fewTableRows;
}

/** Parses a text into the render tree. */
export function parseMarkdown(text: string): RenderedMarkdown {
  if (exceedsSize(text)) {
    return { kind: 'plain', text };
  }
  let tree: Root;
  try {
    tree = fromMarkdown(text, { extensions: [gfm()], mdastExtensions: [gfmFromMarkdown()] });
  } catch {
    // The parser accepts every text, but its tree builder can run out of
    // stack on extreme nesting; the text still shows.
    return { kind: 'plain', text };
  }
  const context: Context = { definitions: definitions(tree), depth: 0, inLink: false, source: text };
  try {
    return { kind: 'markdown', nodes: paragraphsAround(convertAll(tree.children, context)) };
  } catch (error) {
    const isTooDeep = error instanceof TooDeep;
    const isOutOfStack = error instanceof RangeError;
    if (isTooDeep || isOutOfStack) {
      return { kind: 'plain', text };
    }
    throw error;
  }
}

const INLINE_KINDS = new Set<RenderNode['kind']>([
  'break',
  'delete',
  'emphasis',
  'image',
  'inline_code',
  'link',
  'strong',
  'text',
]);

/**
 * Puts inline nodes at the top level, such as a block of raw HTML shown as
 * text, into paragraphs, so that every text sits in a block.
 */
function paragraphsAround(nodes: RenderNode[]): RenderNode[] {
  const blocks: RenderNode[] = [];
  let inline: RenderNode[] = [];
  for (const node of nodes) {
    if (INLINE_KINDS.has(node.kind)) {
      inline.push(node);
      continue;
    }
    if (inline.length > 0) {
      blocks.push({ children: inline, kind: 'paragraph' });
      inline = [];
    }
    blocks.push(node);
  }
  if (inline.length > 0) {
    blocks.push({ children: inline, kind: 'paragraph' });
  }
  return blocks;
}

/** The URLs of a tree's link reference definitions, by their normalized label. */
function definitions(tree: Root): Map<string, string> {
  const found = new Map<string, string>();
  const pending: Nodes[] = [tree];
  for (let node = pending.pop(); node !== undefined; node = pending.pop()) {
    // The first definition of a label wins, as in CommonMark.
    if (node.type === 'definition') {
      if (!found.has(node.identifier)) {
        found.set(node.identifier, node.url);
      }
    }
    if ('children' in node) {
      pending.push(...node.children);
    }
  }
  return found;
}

function deeper(context: Context): Context {
  if (context.depth >= MAX_NESTING) {
    throw new TooDeep();
  }
  return { ...context, depth: context.depth + 1 };
}

function text(value: string): RenderNode {
  return { kind: 'text', value };
}

/** The text a node was written as. */
function sourceOf(node: Nodes, context: Context): string {
  const start = node.position?.start.offset;
  const end = node.position?.end.offset;
  if (start === undefined) {
    return '';
  }
  return end === undefined ? '' : context.source.slice(start, end);
}

function convertAll(nodes: readonly Nodes[], context: Context): RenderNode[] {
  return nodes.flatMap((node) => convert(node, context));
}

function children(node: { children: readonly Nodes[] }, context: Context): RenderNode[] {
  return convertAll(node.children, deeper(context));
}

function convert(node: Nodes | RootContent, context: Context): RenderNode[] {
  switch (node.type) {
    case 'blockquote':
    case 'delete':
    case 'emphasis':
    case 'paragraph':
    case 'strong': {
      return [{ children: children(node, context), kind: node.type }];
    }
    case 'break': {
      return [{ kind: 'break' }];
    }
    case 'code': {
      return [{ kind: 'code', language: node.lang ?? null, value: node.value }];
    }
    case 'definition': {
      return [{ children: [text(sourceOf(node, context))], kind: 'paragraph' }];
    }
    case 'footnoteDefinition': {
      const label = text(`[^${node.label ?? node.identifier}]: `);
      return [{ children: [label], kind: 'paragraph' }, ...children(node, context)];
    }
    case 'footnoteReference': {
      return [text(`[^${node.label ?? node.identifier}]`)];
    }
    case 'heading': {
      return [{ children: children(node, context), kind: 'heading', level: node.depth }];
    }
    case 'html': {
      // Raw HTML shows as the text it is.
      return [text(node.value)];
    }
    case 'image': {
      return [image(node.url, node.alt ?? '', context)];
    }
    case 'imageReference': {
      const url = context.definitions.get(node.identifier);
      return url === undefined ? [text(sourceOf(node, context))] : [image(url, node.alt ?? '', context)];
    }
    case 'inlineCode': {
      return [{ kind: 'inline_code', value: node.value }];
    }
    case 'link': {
      return [link(node.url, node, context)];
    }
    case 'linkReference': {
      const url = context.definitions.get(node.identifier);
      return url === undefined ? children(node, context) : [link(url, node, context)];
    }
    case 'list': {
      const inner = deeper(context);
      const items = node.children.map((item) => ({ checked: item.checked ?? null, children: children(item, inner) }));
      return [{ items, kind: 'list', ordered: node.ordered ?? false, start: node.start ?? null }];
    }
    case 'table': {
      const inner = deeper(context);
      const rows = node.children.map((row) => row.children.map((cell) => children(cell, inner)));
      const [head = [], ...body] = rows;
      return [{ align: node.align ?? [], body, head, kind: 'table' }];
    }
    case 'text': {
      return [text(node.value)];
    }
    case 'thematicBreak': {
      return [{ kind: 'thematic_break' }];
    }
    case 'listItem':
    case 'root':
    case 'tableCell':
    case 'tableRow':
    case 'yaml': {
      // Lists and tables convert their own items, rows and cells, and the
      // parser makes no front matter; should one appear here, it shows as the
      // text it was written as.
      return [text(sourceOf(node, context))];
    }
  }
}

function link(url: string, node: { children: readonly Nodes[] }, context: Context): RenderNode {
  return { children: children(node, { ...context, inLink: true }), kind: 'link', url };
}

function image(url: string, description: string, context: Context): RenderNode {
  return context.inLink ? text(description) : { description, kind: 'image', url };
}

const FENCE = /^ {0,3}(`{3,}|~{3,})/u;

interface Fence {
  readonly char: string;
  readonly length: number;
}

/** The fence that a fence line leaves open: it opens one, closes `open`, or leaves it. */
function nextFence(open: Fence | null, line: string): Fence | null {
  const marker = FENCE.exec(line)?.[1];
  if (marker === undefined) {
    return open;
  }
  const char = marker.charAt(0);
  if (open === null) {
    return { char, length: marker.length };
  }
  const isSameChar = char === open.char;
  const isLongEnough = marker.length >= open.length;
  return isSameChar && isLongEnough ? null : open;
}

/**
 * Where the finished blocks of a streaming text end: after the last blank
 * line outside a fenced code block. The text before it no longer changes its
 * meaning as more arrives, so the renderer parses it once.
 */
export function finishedBlocksEnd(text: string): number {
  let fence: Fence | null = null;
  let end = 0;
  let offset = 0;
  const lines = text.split('\n');
  // The last line may still grow, so it never ends a block.
  for (const line of lines.slice(0, -1)) {
    fence = nextFence(fence, line);
    offset += line.length + 1;
    const isOutsideFence = fence === null;
    const isBlank = line.trim() === '';
    if (isOutsideFence && isBlank) {
      end = offset;
    }
  }
  return end;
}

/**
 * Parses a text that grows while a reply streams. The finished blocks are
 * parsed once and kept; only the text after them is parsed again.
 */
export class StreamingMarkdown {
  #finished = '';
  #finishedNodes: RenderNode[] = [];

  /** The render tree of the text as it is now. */
  update(text: string): RenderedMarkdown {
    const continuesFinished = text.startsWith(this.#finished);
    const isTooLarge = exceedsSize(text);
    if (!continuesFinished || isTooLarge) {
      this.#finished = '';
      this.#finishedNodes = [];
    }
    if (isTooLarge) {
      return { kind: 'plain', text };
    }
    const end = finishedBlocksEnd(text);
    if (end > this.#finished.length) {
      const block = parseMarkdown(text.slice(this.#finished.length, end));
      if (block.kind === 'plain') {
        return parseMarkdown(text);
      }
      this.#finishedNodes = [...this.#finishedNodes, ...block.nodes];
      this.#finished = text.slice(0, end);
    }
    const rest = parseMarkdown(text.slice(this.#finished.length));
    if (rest.kind === 'plain') {
      return { kind: 'plain', text };
    }
    return { kind: 'markdown', nodes: [...this.#finishedNodes, ...rest.nodes] };
  }
}
