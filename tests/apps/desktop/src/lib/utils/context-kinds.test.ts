// How attached context shows through its kind, or with generic texts without
// one (ADR-0046, decision 6), and the excerpts that the replies' cite links
// name (ADR-0049, decision 4).
import type { Component } from 'svelte';
import { describe, expect, it } from 'vitest';

import type { ContextView, TurnView } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import type { ContextKind } from '../../../../../../apps/desktop/src/lib/types/shell-slots.js';
import { citedItems, citeTarget, findCited } from '../../../../../../apps/desktop/src/lib/utils/citations.js';
import {
  attachmentNoun,
  contextFailure,
  contextIcon,
  contextRefusal,
  contextSummary,
  labelList,
  locationText,
} from '../../../../../../apps/desktop/src/lib/utils/context-kinds.js';

/** A stand-in for an icon component. */
const BOOK_ICON = (() => undefined) as unknown as Component;

const NOTEBOOKS: ContextKind = {
  failure: (code) => (code === 'server_unreachable' ? 'The embedding server could not be reached.' : null),
  found: (labels, count) => `Searched ${labels.join(' and ')}: ${String(count)} passages`,
  gathering: () => 'Searching notebooks…',
  icon: BOOK_ICON,
  id: 'notebooks',
  noun: () => 'Notebook',
};

function view(items: number, failure: string | null): ContextView {
  return {
    attachments: [
      { key: 'n1', label: 'Physics' },
      { key: 'n2', label: 'Optics' },
    ],
    failure,
    items: Array.from({ length: items }, (_unused, index) => ({
      key: `s1:${String(index)}`,
      location: { kind: 'page', number: index + 1 },
      number: index + 1,
      text: 'Light bends.',
      title: 'Optics.pdf',
    })),
    provider: 'notebooks',
  };
}

describe('the words of attached context', () => {
  it('come from the kind', () => {
    expect(contextIcon(NOTEBOOKS)).toBe(BOOK_ICON);
    expect(attachmentNoun(NOTEBOOKS)).toBe('Notebook');
    expect(contextSummary(NOTEBOOKS, view(5, null))).toBe('Searched Physics and Optics: 5 passages');
    expect(contextFailure(NOTEBOOKS, 'server_unreachable')).toBe('The embedding server could not be reached.');
    expect(contextRefusal(NOTEBOOKS, 'server_unreachable')).toBe('The embedding server could not be reached.');
  });

  it('are generic without a kind, as while its plugin is off', () => {
    expect(contextIcon(undefined)).not.toBe(BOOK_ICON);
    expect(attachmentNoun(undefined)).toBe('Attachment');
    expect(contextSummary(undefined, view(1, null))).toBe('Searched Physics and Optics: 1 excerpt');
    expect(contextSummary(undefined, view(3, null))).toBe('Searched Physics and Optics: 3 excerpts');
    expect(contextFailure(undefined, 'server_unreachable')).toBe('The search failed.');
    expect(contextRefusal(undefined, 'notebook_not_found')).toBe('Its plugin does not know it.');
  });

  it('say why a part holds nothing, with the core’s own failures in its words', () => {
    expect(contextSummary(NOTEBOOKS, view(0, null))).toBe('Searched Physics and Optics: nothing applies');
    expect(contextSummary(NOTEBOOKS, view(0, 'plugin_off'))).toBe(
      'Could not search Physics and Optics. Its plugin was off.',
    );
    expect(contextSummary(NOTEBOOKS, view(0, 'timed_out'))).toBe(
      'Could not search Physics and Optics. The search took too long.',
    );
    expect(contextSummary(NOTEBOOKS, view(0, 'server_unreachable'))).toBe(
      'Could not search Physics and Optics. The embedding server could not be reached.',
    );
  });

  it('name an excerpt’s place by its page or its headings', () => {
    expect(locationText({ kind: 'page', number: 3 })).toBe('Page 3');
    expect(locationText({ kind: 'headings', path: ['Waves', 'Energy'] })).toBe('Waves › Energy');
    expect(locationText(null)).toBeNull();
    expect(labelList(['Physics', 'Optics', 'Waves'])).toBe('Physics, Optics, and Waves');
  });
});

describe('the excerpts that cite links name (ADR-0049, decision 4)', () => {
  it('are found by provider and key in any turn of the conversation', () => {
    const part = (id: string, context: ContextView | null): TurnView['parts'][number] => ({
      content: context === null ? { kind: 'user_text', text: 'Why?' } : { context, kind: 'context' },
      createdAt: 0,
      id,
      position: 0,
      runId: null,
      state: 'complete',
      turnId: 't1',
    });
    const turns: TurnView[] = [
      { createdAt: 0, id: 't1', parts: [part('p1', null), part('p2', view(2, null))], position: 0, run: null },
      { createdAt: 0, id: 't2', parts: [part('p3', null)], position: 1, run: null },
    ];

    const cited = citedItems(turns);

    expect(findCited(cited, 'notebooks', 's1:1')?.item.number).toBe(2);
    expect(findCited(cited, 'notebooks', 's1:0')?.provider).toBe('notebooks');
    expect(findCited(cited, 'notebooks', 's1:9')).toBeUndefined();
    expect(findCited(cited, 'wiki', 's1:0')).toBeUndefined();
  });

  it('take a cite link apart into its provider and key, and nothing else', () => {
    expect(citeTarget('cite:notebooks/0199e0f2-aaaa:12')).toEqual({ key: '0199e0f2-aaaa:12', provider: 'notebooks' });
    expect(citeTarget('cite:notebooks/a/b')).toEqual({ key: 'a/b', provider: 'notebooks' });
    for (const url of ['https://example.com', 'cite:', 'cite:notebooks', 'cite:/s1:0', 'cite:notebooks/', 'Cite:x/y']) {
      expect(citeTarget(url), url).toBeNull();
    }
  });
});
