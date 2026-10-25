// The words of sources (plugins/notebooks/docs/README.md, sources): a source
// that is indexed while its server fails is paused with the reason, a failed
// one says why and can be tried again only once its text was read, and the
// details, the progress and the places of sections read as the owner reads
// them.
import { formatBytes } from '@mannie/plugin-sdk/format';
import { describe, expect, it } from 'vitest';

import type { SourceView } from '../../../ui/generated/bindings.js';
import {
  canRetry,
  sectionPlaceLabel,
  sourceDetails,
  sourcePassages,
  sourcePhase,
  sourcePhaseLabel,
  sourceProblem,
  sourceProgress,
} from '../../../ui/utils/notebooks-labels.js';

const SOURCE: SourceView = {
  addedAt: 0,
  bytes: 2_048,
  embedded: 3,
  failure: null,
  id: 'source-1',
  included: true,
  indexedAt: null,
  kind: 'pdf',
  name: 'notes.pdf',
  notebookId: 'notebook-1',
  pages: 12,
  passages: 10,
  state: 'indexing',
};

describe('the words of a source', () => {
  it('say that an indexed source whose server failed is paused, and why', () => {
    const paused = { ...SOURCE, failure: 'server_unreachable' };

    expect(sourcePhase(SOURCE)).toBe('indexing');
    expect(sourcePhase(paused)).toBe('paused');
    expect(sourcePhaseLabel('paused')).toBe('Paused');
    expect(sourceProblem(SOURCE)).toBeNull();
    expect(sourceProblem(paused)).toBe(
      'The embedding server could not be reached. Mannie tries again in a few minutes.',
    );
  });

  it('say why a source failed, which can be tried again once its text was read', () => {
    const failedIndexing = { ...SOURCE, failure: 'server_error', state: 'failed' as const };
    const failedReading = { ...SOURCE, failure: 'no_text', passages: 0, state: 'failed' as const };

    expect(sourceProblem(failedReading)).toBe('This PDF has no text. It may be a scan.');
    expect(canRetry(failedIndexing)).toBe(true);
    expect(canRetry(failedReading)).toBe(false);
    expect(canRetry({ ...SOURCE, state: 'ready' })).toBe(false);
  });

  it('give the kind, size and pages, and the progress of the indexing', () => {
    // Sizes follow the system's locale, as the core formats them.
    expect(sourceDetails(SOURCE)).toBe(`PDF · ${formatBytes(2_048)} · 12 pages`);
    expect(sourceDetails({ ...SOURCE, kind: 'markdown', pages: null })).toBe(`Markdown · ${formatBytes(2_048)}`);
    expect(sourceProgress(SOURCE)).toBe('3 of 10 passages');
    expect(sourcePassages({ ...SOURCE, passages: 1 })).toBe('1 passage');
  });

  it("name a section's place by its page or its headings, and nothing for a whole text", () => {
    expect(sectionPlaceLabel({ kind: 'page', number: 3 })).toBe('Page 3');
    expect(sectionPlaceLabel({ kind: 'headings', path: ['Optics', 'Lenses'] })).toBe('Optics › Lenses');
    expect(sectionPlaceLabel({ kind: 'headings', path: [] })).toBeNull();
    expect(sectionPlaceLabel({ kind: 'whole' })).toBeNull();
  });
});
