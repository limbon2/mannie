// Pure intra-line word diff highlighter, side-by-side split-row aligner, and
// unified hunk patch builder for per-hunk staging, unstaging and discarding.
import type { Hunk, Line } from '../generated/bindings.js';

/** A span of text inside a diff line, optionally highlighted as changed. */
export interface DiffSegment {
  readonly emphasized: boolean;
  readonly text: string;
}

/** A unified diff line annotated with optional intra-line highlight segments. */
export interface AnnotatedLine {
  readonly line: Line;
  readonly segments: readonly DiffSegment[];
}

/** One aligned row in side-by-side split diff mode. */
export interface SplitRow {
  readonly left: AnnotatedLine | null;
  readonly right: AnnotatedLine | null;
}

/** Maximum line length for intra-line LCS diffing to avoid lag on minified files. */
const MAX_WORD_DIFF_CHARS = 2000;

/** Maximum tokens per line for O(N*M) LCS table. */
const MAX_WORD_DIFF_TOKENS = 220;

/** Splits text into word, whitespace and punctuation tokens so highlights align cleanly. */
function tokenize(text: string): string[] {
  const matches = text.match(/[\p{L}\p{N}_]+|\s+|[^\p{L}\p{N}_\s]/gu);
  return matches ?? [];
}

/** Merges consecutive segments with the same `emphasized` flag. */
function coalesce(segments: readonly DiffSegment[]): DiffSegment[] {
  const result: DiffSegment[] = [];
  for (const segment of segments) {
    if (segment.text.length === 0) {
      continue;
    }
    const last = result.at(-1);
    if (last?.emphasized === segment.emphasized) {
      result[result.length - 1] = {
        emphasized: last.emphasized,
        text: last.text + segment.text,
      };
    } else {
      result.push(segment);
    }
  }
  return result;
}

/** Highlight segments for a paired removed and added diff line. */
export interface LineWordDiff {
  readonly added: readonly DiffSegment[];
  readonly removed: readonly DiffSegment[];
}

/** Computes intra-line highlight spans between a removed line and its paired added line. */
export function diffLineWords(removedText: string, addedText: string): LineWordDiff {
  if (removedText === addedText) {
    const plain = removedText.length === 0 ? [] : [{ emphasized: false, text: removedText }];
    return { added: plain, removed: plain };
  }
  const isRemovedTooLong = removedText.length > MAX_WORD_DIFF_CHARS;
  const isAddedTooLong = addedText.length > MAX_WORD_DIFF_CHARS;
  if (isRemovedTooLong || isAddedTooLong) {
    return {
      added: [{ emphasized: false, text: addedText }],
      removed: [{ emphasized: false, text: removedText }],
    };
  }

  const left = tokenize(removedText);
  const right = tokenize(addedText);
  const hasTooManyLeft = left.length > MAX_WORD_DIFF_TOKENS;
  const hasTooManyRight = right.length > MAX_WORD_DIFF_TOKENS;
  if (hasTooManyLeft || hasTooManyRight) {
    return {
      added: [{ emphasized: false, text: addedText }],
      removed: [{ emphasized: false, text: removedText }],
    };
  }

  const rows = left.length + 1;
  const cols = right.length + 1;
  const table = new Uint16Array(rows * cols);

  for (let i = left.length - 1; i >= 0; i -= 1) {
    for (let j = right.length - 1; j >= 0; j -= 1) {
      const idx = i * cols + j;
      if (left[i] === right[j]) {
        table[idx] = (table[(i + 1) * cols + (j + 1)] ?? 0) + 1;
      } else {
        const down = table[(i + 1) * cols + j] ?? 0;
        const across = table[i * cols + (j + 1)] ?? 0;
        table[idx] = down >= across ? down : across;
      }
    }
  }

  const removedSegs: DiffSegment[] = [];
  const addedSegs: DiffSegment[] = [];
  let i = 0;
  let j = 0;

  while (i < left.length) {
    if (j >= right.length) {
      break;
    }
    const leftTok = left[i];
    const rightTok = right[j];
    if (leftTok === rightTok) {
      removedSegs.push({ emphasized: false, text: leftTok });
      addedSegs.push({ emphasized: false, text: rightTok });
      i += 1;
      j += 1;
    } else if ((table[(i + 1) * cols + j] ?? 0) >= (table[i * cols + (j + 1)] ?? 0)) {
      removedSegs.push({ emphasized: true, text: leftTok });
      i += 1;
    } else {
      addedSegs.push({ emphasized: true, text: rightTok });
      j += 1;
    }
  }

  while (i < left.length) {
    removedSegs.push({ emphasized: true, text: left[i] });
    i += 1;
  }
  while (j < right.length) {
    addedSegs.push({ emphasized: true, text: right[j] });
    j += 1;
  }

  return {
    added: coalesce(addedSegs),
    removed: coalesce(removedSegs),
  };
}

/** Annotates lines in a hunk with word-level emphasis on paired removed/added blocks. */
export function annotateHunkLines(lines: readonly Line[]): AnnotatedLine[] {
  const result: AnnotatedLine[] = [];
  let index = 0;

  while (index < lines.length) {
    const current = lines[index];
    if (current.kind !== 'removed') {
      result.push({
        line: current,
        segments: [{ emphasized: false, text: current.text }],
      });
      index += 1;
      continue;
    }

    const removedBlock: Line[] = [];
    while (index < lines.length) {
      const item = lines[index];
      if (item.kind !== 'removed') {
        break;
      }
      removedBlock.push(item);
      index += 1;
    }

    const addedBlock: Line[] = [];
    while (index < lines.length) {
      const item = lines[index];
      if (item.kind !== 'added') {
        break;
      }
      addedBlock.push(item);
      index += 1;
    }

    const pairCount = Math.min(removedBlock.length, addedBlock.length);
    const removedAnnotated: AnnotatedLine[] = [];
    const addedAnnotated: AnnotatedLine[] = [];

    for (let k = 0; k < pairCount; k += 1) {
      const rem = removedBlock[k];
      const add = addedBlock[k];
      const { added, removed } = diffLineWords(rem.text, add.text);
      removedAnnotated.push({ line: rem, segments: removed });
      addedAnnotated.push({ line: add, segments: added });
    }

    for (let k = pairCount; k < removedBlock.length; k += 1) {
      const rem = removedBlock[k];
      removedAnnotated.push({ line: rem, segments: [{ emphasized: false, text: rem.text }] });
    }
    for (let k = pairCount; k < addedBlock.length; k += 1) {
      const add = addedBlock[k];
      addedAnnotated.push({ line: add, segments: [{ emphasized: false, text: add.text }] });
    }

    result.push(...removedAnnotated, ...addedAnnotated);
  }

  return result;
}

/** Builds aligned side-by-side rows from a hunk's lines with word-level emphasis. */
export function buildSplitRows(lines: readonly Line[]): SplitRow[] {
  const rows: SplitRow[] = [];
  let index = 0;

  while (index < lines.length) {
    const current = lines[index];
    if (current.kind === 'context') {
      const annotated: AnnotatedLine = {
        line: current,
        segments: [{ emphasized: false, text: current.text }],
      };
      rows.push({ left: annotated, right: annotated });
      index += 1;
      continue;
    }

    const removedBlock: Line[] = [];
    while (index < lines.length) {
      const item = lines[index];
      if (item.kind !== 'removed') {
        break;
      }
      removedBlock.push(item);
      index += 1;
    }

    const addedBlock: Line[] = [];
    while (index < lines.length) {
      const item = lines[index];
      if (item.kind !== 'added') {
        break;
      }
      addedBlock.push(item);
      index += 1;
    }

    const maxLen = Math.max(removedBlock.length, addedBlock.length);
    for (let k = 0; k < maxLen; k += 1) {
      const rem: Line | null = k < removedBlock.length ? removedBlock[k] : null;
      const add: Line | null = k < addedBlock.length ? addedBlock[k] : null;
      const hasBoth = rem !== null && add !== null;
      if (hasBoth) {
        const { added, removed } = diffLineWords(rem.text, add.text);
        rows.push({
          left: { line: rem, segments: removed },
          right: { line: add, segments: added },
        });
      } else if (rem !== null) {
        rows.push({
          left: { line: rem, segments: [{ emphasized: false, text: rem.text }] },
          right: null,
        });
      } else if (add !== null) {
        rows.push({
          left: null,
          right: { line: add, segments: [{ emphasized: false, text: add.text }] },
        });
      }
    }
  }

  return rows;
}

/** Returns the unified diff prefix for a hunk line kind. */
function linePrefix(kind: Line['kind']): string {
  if (kind === 'added') {
    return '+';
  }
  if (kind === 'removed') {
    return '-';
  }
  return ' ';
}

/** Builds a valid single-hunk unified diff patch for `git apply`. */
export function buildHunkPatch(path: string, hunk: Hunk, originalPath?: string | null): string {
  const fromPath = originalPath ?? path;
  const lines: string[] = [`diff --git a/${fromPath} b/${path}`, `--- a/${fromPath}`, `+++ b/${path}`, hunk.header];
  for (const line of hunk.lines) {
    lines.push(`${linePrefix(line.kind)}${line.text}`);
    if (line.noNewline) {
      lines.push('\\ No newline at end of file');
    }
  }
  return `${lines.join('\n')}\n`;
}
