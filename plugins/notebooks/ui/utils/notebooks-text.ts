// A section of a source's text as the side panel shows it (plugins/notebooks/
// docs/README.md, answers): in pieces, with the passage that a citation
// opened marked, and the window of sections that one read returns.
import { MAX_SECTIONS_PER_READ } from '../generated/plugin.js';

/** A piece of a section's text, which the panel marks when it is the passage. */
export interface TextSegment {
  readonly marked: boolean;
  readonly text: string;
}

/** Where a passage stands in its section's text, in UTF-16 offsets, as the webview counts them. */
export interface TextRange {
  readonly end: number;
  readonly start: number;
}

/** A number kept between `low` and `high`. */
function clamp(value: number, low: number, high: number): number {
  return Math.min(Math.max(value, low), high);
}

/** A section's text in pieces, with the passage in `range` marked, or as one piece without a range. */
export function textSegments(text: string, range: TextRange | null): TextSegment[] {
  if (range === null) {
    return [{ marked: false, text }];
  }
  const start = clamp(range.start, 0, text.length);
  const end = clamp(range.end, start, text.length);
  const segments: TextSegment[] = [
    { marked: false, text: text.slice(0, start) },
    { marked: true, text: text.slice(start, end) },
    { marked: false, text: text.slice(end) },
  ];
  // The marked piece stays even when it is empty, so the panel still scrolls to its place.
  return segments.filter((segment) => {
    const hasText = segment.text !== '';
    return segment.marked || hasText;
  });
}

/** The first section of the window of sections that holds a section. */
export function windowStart(section: number): number {
  return Math.floor(section / MAX_SECTIONS_PER_READ) * MAX_SECTIONS_PER_READ;
}
