// The words of sources as the plugin's views show them (plugins/notebooks/
// docs/README.md, sources): what a source is, its size and pages, where it is
// in its life with why it failed or waits, its progress, and where a section
// of its text stands.
import { formatBytes, formatCount } from '@mannie/plugin-sdk/format';

import type { SectionPlace, SourceKind, SourceView } from '../generated/bindings.js';
import { m } from '../generated/messages.js';

import { sourceFailureMessage } from './notebooks-messages.js';

/**
 * Where a source is in its life as the owner reads it: a source that is
 * indexed and waits after its server failed is paused.
 */
export type SourcePhase = 'failed' | 'indexing' | 'paused' | 'reading' | 'ready' | 'waiting';

/** What a source is, such as "PDF". */
export function sourceKindLabel(kind: SourceKind): string {
  switch (kind) {
    case 'markdown': {
      return m.notebooks_source_kind_markdown();
    }
    case 'pdf': {
      return m.notebooks_source_kind_pdf();
    }
    case 'text': {
      return m.notebooks_source_kind_text();
    }
  }
}

/** The source's kind, size and pages, such as "PDF · 2.4 MB · 12 pages". */
export function sourceDetails(source: SourceView): string {
  const parts = [sourceKindLabel(source.kind), formatBytes(source.bytes)];
  if (source.pages !== null) {
    parts.push(m.notebooks_source_pages({ count: source.pages }));
  }
  return parts.join(' · ');
}

/** Where a source is in its life. */
export function sourcePhase(source: SourceView): SourcePhase {
  const isPaused = source.state === 'indexing' && source.failure !== null;
  return isPaused ? 'paused' : source.state;
}

/** The words of a source's phase, such as "Ready". */
export function sourcePhaseLabel(phase: SourcePhase): string {
  switch (phase) {
    case 'failed': {
      return m.notebooks_source_state_failed();
    }
    case 'indexing': {
      return m.notebooks_source_state_indexing();
    }
    case 'paused': {
      return m.notebooks_source_state_paused();
    }
    case 'reading': {
      return m.notebooks_source_state_reading();
    }
    case 'ready': {
      return m.notebooks_source_state_ready();
    }
    case 'waiting': {
      return m.notebooks_source_state_waiting();
    }
  }
}

/** Why a source failed or why its indexing waits, or `null` while nothing went wrong. */
export function sourceProblem(source: SourceView): string | null {
  const reason = sourceFailureMessage(source.failure ?? '');
  switch (sourcePhase(source)) {
    case 'failed': {
      return reason;
    }
    case 'paused': {
      return m.notebooks_source_paused_description({ reason });
    }
    case 'indexing':
    case 'reading':
    case 'ready':
    case 'waiting': {
      return null;
    }
  }
}

/** How far a source's indexing is, such as "12 of 40 passages". */
export function sourceProgress(source: SourceView): string {
  return m.notebooks_source_progress({ embedded: formatCount(source.embedded), total: formatCount(source.passages) });
}

/** The size of a source's search, such as "40 passages". */
export function sourcePassages(source: SourceView): string {
  return m.notebooks_source_passages({ count: source.passages });
}

/**
 * Whether a source can be tried again: one that failed while it was
 * indexed. One that failed while it was read has lost its bytes.
 */
export function canRetry(source: SourceView): boolean {
  const failed = source.state === 'failed';
  const wasRead = source.passages > 0;
  return failed && wasRead;
}

/** Where a section stands, such as "Page 3" or "Optics › Lenses", or `null` for the whole of a text. */
export function sectionPlaceLabel(place: SectionPlace): string | null {
  switch (place.kind) {
    case 'headings': {
      return place.path.length === 0 ? null : place.path.join(' › ');
    }
    case 'page': {
      return m.notebooks_page({ page: place.number });
    }
    case 'whole': {
      return null;
    }
  }
}
