// Where the plugin shows things: a passage's key names its source, a
// citation's passage opens the viewer of its source, which then marks it, a
// source opened from its notebook shows from its start, and a notebook's page
// may replace the address that led to it.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { panelRows } from '../../../ui/state/panel-rows.svelte.js';
import { sourceViewer } from '../../../ui/state/source-viewer.svelte.js';
import { openNotebook, openPassage, openSource, sourceOfPassage } from '../../../ui/utils/notebooks-places.js';

const navigation = vi.hoisted(() => ({ openView: vi.fn(), viewParameters: vi.fn() }));

vi.mock('@mannie/plugin-sdk/navigation', () => navigation);

beforeEach(() => {
  navigation.openView.mockReset();
  navigation.openView.mockResolvedValue(undefined);
  sourceViewer.close();
  sourceViewer.request = null;
});

describe('the places of the plugin', () => {
  it("read a passage's source from its key", () => {
    expect(sourceOfPassage('0199a1b2-source:12')).toBe('0199a1b2-source');
    expect(sourceOfPassage('a:b:3')).toBe('a:b');
    expect(['notebook-1', ':3', 'source:', 'source:x'].map(sourceOfPassage)).toEqual([null, null, null, null]);
  });

  it('open a passage in the viewer of its source, which marks it', () => {
    openPassage('source-1:4');

    expect(sourceViewer.open).toBe(true);
    expect(sourceViewer.request).toEqual({ key: 'source-1:4', opener: null, sourceId: 'source-1' });
  });

  it('open a source from its start, and nothing for a key of another shape', () => {
    // The viewer only keeps the element, which the dialog gives the focus back to.
    const opener = { isConnected: true } as HTMLElement;
    openPassage('source-1:4');
    const first = sourceViewer.request;

    openSource('source-1', opener);
    const fromStart = sourceViewer.request;
    sourceViewer.close();
    openPassage('notebook-1');

    expect(fromStart).toEqual({ key: null, opener, sourceId: 'source-1' });
    expect(fromStart).not.toBe(first);
    expect(sourceViewer.open).toBe(false);
  });

  it("open a notebook's page with its row of the sidebar, replacing the address when asked", async () => {
    panelRows.set('notebook-1', false);
    await openNotebook('notebook-1');
    await openNotebook('notebook-2', { replace: true });

    expect([panelRows.isOpen('notebook-1'), panelRows.isOpen('notebook-2')]).toEqual([true, true]);
    expect(navigation.openView.mock.calls).toEqual([
      ['notebooks', { notebook: 'notebook-1' }, undefined],
      ['notebooks', { notebook: 'notebook-2' }, { replace: true }],
    ]);
  });
});
