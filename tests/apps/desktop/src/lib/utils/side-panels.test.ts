// The side panels' address (ADR-0025, decision 4): the open panels and the
// visible one live in the main view's search parameters.
import { describe, expect, it } from 'vitest';

import {
  MAX_OPEN_PANELS,
  openPanel,
  PANEL_PARAMETER,
  parsePanel,
  readSidePanels,
  SHOWN_PANEL_PARAMETER,
  type SidePanelState,
  withPanelClosed,
  withPanelOpened,
  withPanelShown,
  withSidePanels,
} from '../../../../../../apps/desktop/src/lib/utils/side-panels.js';

function url(search: string): URL {
  return new URL(`https://tauri.localhost/${search}`);
}

function keys(state: SidePanelState): string[] {
  return state.panels.map((panel) => panel.key);
}

const ONE = openPanel('chat.conversation', 'one');
const TWO = openPanel('chat.conversation', 'two');
const THREE = openPanel('chat.conversation', 'three');

describe('parsePanel', () => {
  it('splits an address at its first colon into the kind and the argument', () => {
    expect(parsePanel('chat.conversation:0199e0f2-0000-7000-8000-000000000001')).toEqual({
      argument: '0199e0f2-0000-7000-8000-000000000001',
      key: 'chat.conversation:0199e0f2-0000-7000-8000-000000000001',
      kind: 'chat.conversation',
    });
    expect(parsePanel('example:a:b')?.argument).toBe('a:b');
  });

  it('refuses an address without a kind, an argument or a reasonable length', () => {
    expect(parsePanel('no-colon')).toBeNull();
    expect(parsePanel(':argument')).toBeNull();
    expect(parsePanel('kind:')).toBeNull();
    expect(parsePanel(`kind:${'x'.repeat(600)}`)).toBeNull();
  });
});

describe('readSidePanels', () => {
  it('reads no panels from an address without them', () => {
    expect(readSidePanels(url('?view=chat'))).toEqual({ panels: [], shown: null });
  });

  it('reads the panels in the order of their tabs, without repeats or invalid ones', () => {
    const search = new URLSearchParams();
    for (const value of [ONE.key, 'broken', TWO.key, ONE.key]) {
      search.append(PANEL_PARAMETER, value);
    }
    search.set(SHOWN_PANEL_PARAMETER, ONE.key);

    const state = readSidePanels(url(`?${search.toString()}`));

    expect(keys(state)).toEqual([ONE.key, TWO.key]);
    expect(state.shown).toBe(ONE.key);
  });

  it('shows the last tab when the address names no open panel to show', () => {
    const search = new URLSearchParams([
      [PANEL_PARAMETER, ONE.key],
      [PANEL_PARAMETER, TWO.key],
      [SHOWN_PANEL_PARAMETER, THREE.key],
    ]);

    expect(readSidePanels(url(`?${search.toString()}`)).shown).toBe(TWO.key);
  });

  it('keeps the newest panels when the address holds too many', () => {
    const search = new URLSearchParams();
    for (let index = 0; index < MAX_OPEN_PANELS + 2; index += 1) {
      search.append(PANEL_PARAMETER, `example:${String(index)}`);
    }

    const state = readSidePanels(url(`?${search.toString()}`));

    expect(state.panels).toHaveLength(MAX_OPEN_PANELS);
    expect(state.panels[0]?.argument).toBe('2');
  });
});

describe('withSidePanels', () => {
  it('replaces the panels and keeps every other parameter', () => {
    const state: SidePanelState = { panels: [ONE, TWO], shown: TWO.key };

    const search = withSidePanels(`?view=chat&conversation=abc&${PANEL_PARAMETER}=old%3Aone`, state);

    expect(search.get('view')).toBe('chat');
    expect(search.get('conversation')).toBe('abc');
    expect(search.getAll(PANEL_PARAMETER)).toEqual([ONE.key, TWO.key]);
    expect(search.get(SHOWN_PANEL_PARAMETER)).toBe(TWO.key);
    expect(readSidePanels(url(`?${search.toString()}`))).toEqual(state);
  });

  it('removes the panel parameters when no panel is open', () => {
    const search = withSidePanels(`?view=chat&${PANEL_PARAMETER}=a%3Ab&${SHOWN_PANEL_PARAMETER}=a%3Ab`, {
      panels: [],
      shown: null,
    });

    expect(search.toString()).toBe('view=chat');
  });
});

describe('opening, showing and closing', () => {
  it('opens a new panel at the end and shows it', () => {
    const state = withPanelOpened({ panels: [ONE], shown: ONE.key }, TWO);

    expect(keys(state)).toEqual([ONE.key, TWO.key]);
    expect(state.shown).toBe(TWO.key);
  });

  it('shows a panel that is open already instead of opening it twice', () => {
    const state = withPanelOpened({ panels: [ONE, TWO], shown: TWO.key }, ONE);

    expect(keys(state)).toEqual([ONE.key, TWO.key]);
    expect(state.shown).toBe(ONE.key);
  });

  it('closes the oldest panel when another opens beyond the limit', () => {
    const panels = Array.from({ length: MAX_OPEN_PANELS }, (_, index) => openPanel('example', String(index)));

    const state = withPanelOpened({ panels, shown: panels[0]?.key ?? null }, THREE);

    expect(state.panels).toHaveLength(MAX_OPEN_PANELS);
    expect(state.panels[0]?.argument).toBe('1');
    expect(state.shown).toBe(THREE.key);
  });

  it('shows only a panel that is open', () => {
    const state: SidePanelState = { panels: [ONE, TWO], shown: TWO.key };

    expect(withPanelShown(state, ONE.key).shown).toBe(ONE.key);
    expect(withPanelShown(state, THREE.key)).toBe(state);
  });

  it('shows the tab after a closed visible panel, or the one before the last', () => {
    const state: SidePanelState = { panels: [ONE, TWO, THREE], shown: TWO.key };

    expect(withPanelClosed(state, TWO.key)).toEqual({ panels: [ONE, THREE], shown: THREE.key });
    expect(withPanelClosed({ panels: [ONE, TWO], shown: TWO.key }, TWO.key)).toEqual({
      panels: [ONE],
      shown: ONE.key,
    });
    expect(withPanelClosed({ panels: [ONE, TWO], shown: ONE.key }, ONE.key)).toEqual({
      panels: [TWO],
      shown: TWO.key,
    });
    expect(withPanelClosed({ panels: [ONE], shown: ONE.key }, ONE.key)).toEqual({ panels: [], shown: null });
  });

  it('keeps the visible panel when another closes, and ignores a panel that is not open', () => {
    const state: SidePanelState = { panels: [ONE, TWO], shown: TWO.key };

    expect(withPanelClosed(state, ONE.key)).toEqual({ panels: [TWO], shown: TWO.key });
    expect(withPanelClosed(state, THREE.key)).toBe(state);
  });
});
