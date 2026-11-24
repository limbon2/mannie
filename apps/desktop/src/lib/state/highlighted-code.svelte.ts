// Reactive hook that highlights code lazily with Shiki and incremental line
// caching while streaming (ADR-0078). Returns `null` on SSR, while the grammar
// loads, for unsupported languages, or above size bounds so callers render
// plain text without layout shift.
import type { GrammarState } from 'shiki/core';

import {
  type HighlightedLine,
  isHighlightable,
  type LanguageHighlighter,
  loadHighlighter,
  resolveLanguage,
} from '#lib/utils/highlighter.js';

interface LoadedHighlighter {
  readonly highlighter: LanguageHighlighter | null;
  readonly language: string;
}

interface StreamingCache {
  highlighter: LanguageHighlighter | null;
  lines: HighlightedLine[];
  state?: GrammarState;
  text: string;
}

export interface HighlightedCodeState {
  /** Tokenized lines when ready and within bounds, or `null` for plain text. */
  readonly lines: readonly HighlightedLine[] | null;
}

export function useHighlightedCode(value: () => string, language: () => string | null): HighlightedCodeState {
  let loaded = $state.raw<LoadedHighlighter | null>(null);

  // Completed lines already tokenized; streaming appends only re-tokenize new
  // lines starting from the previous grammar state.
  let cache: StreamingCache = { highlighter: null, lines: [], text: '' };

  $effect(() => {
    const resolved = resolveLanguage(language());
    if (resolved === null) {
      loaded = null;
      return;
    }

    let cancelled = false;
    void loadHighlighter(resolved)
      .then((highlighter) => {
        if (!cancelled) {
          loaded = { highlighter, language: resolved };
        }
      })
      .catch(() => {
        if (!cancelled) {
          loaded = { highlighter: null, language: resolved };
        }
      });

    return () => {
      cancelled = true;
    };
  });

  const lines = $derived.by((): readonly HighlightedLine[] | null => {
    const raw = value();
    if (!isHighlightable(raw)) {
      return null;
    }

    const resolved = resolveLanguage(language());
    if (resolved === null) {
      return null;
    }

    const highlighter = loaded?.language === resolved ? loaded.highlighter : null;
    if (highlighter === null) {
      return null;
    }

    // A trailing newline would render as an extra empty line.
    const code = raw.endsWith('\n') ? raw.slice(0, -1) : raw;
    let next = cache;
    const cacheMatches = next.highlighter === highlighter && code.startsWith(next.text);
    if (!cacheMatches) {
      next = { highlighter, lines: [], text: '' };
    }

    const end = code.lastIndexOf('\n') + 1;
    if (end > next.text.length) {
      const result = highlighter.highlight(code.slice(next.text.length, end - 1), next.state);
      next = {
        highlighter,
        lines: [...next.lines, ...result.lines],
        state: result.state,
        text: code.slice(0, end),
      };
    }

    cache = next;
    const tail = code.slice(end);
    const tailLine = highlighter.highlight(tail, next.state).lines.at(0);
    return tailLine === undefined ? next.lines : [...next.lines, tailLine];
  });

  return {
    get lines() {
      return lines;
    },
  };
}
