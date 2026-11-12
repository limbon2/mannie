// The side panels of the window (ADR-0025): the open panels come from the
// main view's address, which changes whenever a panel opens, shows or
// closes. A panel that stays open keeps its object, so its component keeps
// the state that it made of its argument, such as a terminal's view.
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { goto } from '$app/navigation';

import { SidePanels } from '../../../../../../apps/desktop/src/lib/state/side-panels.svelte.js';
import {
  openPanel,
  type SidePanelState,
  withSidePanels,
} from '../../../../../../apps/desktop/src/lib/utils/side-panels.js';

const page = vi.hoisted(() => ({ route: { id: '/' }, url: new URL('http://localhost/') }));

vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));
vi.mock('$app/state', () => ({ page }));

const ONE = openPanel('terminal.session', 'conversation.1');
const TWO = openPanel('terminal.session', 'conversation.2');

/** Puts `state` into the main view's address. */
function address(state: SidePanelState): void {
  page.url = new URL(`http://localhost/?${withSidePanels('view=chat', state).toString()}`);
}

beforeEach(() => {
  vi.clearAllMocks();
  page.route.id = '/';
  page.url = new URL('http://localhost/');
});

describe('SidePanels', () => {
  it('keeps the object of a panel that stays open when another tab shows', () => {
    const panels = new SidePanels();
    address({ panels: [ONE, TWO], shown: TWO.key });
    const before = panels.state;

    address({ panels: [ONE, TWO], shown: ONE.key });
    const after = panels.state;

    expect(after.shown).toBe(ONE.key);
    expect(after.panels).toHaveLength(2);
    expect(after.panels[0]).toBe(before.panels[0]);
    expect(after.panels[1]).toBe(before.panels[1]);
  });

  it('keeps the objects of the panels that stay open when one closes or another opens', () => {
    const panels = new SidePanels();
    address({ panels: [ONE, TWO], shown: TWO.key });
    const [first, second] = panels.state.panels;

    address({ panels: [TWO], shown: TWO.key });
    const afterClosing = panels.state;
    const three = openPanel('git.repository', 'project');
    address({ panels: [TWO, three], shown: three.key });
    const afterOpening = panels.state;

    expect(afterClosing.panels[0]).toBe(second);
    expect(afterOpening.panels[0]).toBe(second);
    expect(afterOpening.panels[1]).toEqual(three);
    expect(afterOpening.panels).not.toContain(first);
  });

  it('replaces a same-kind panel in one navigation', async () => {
    const panels = new SidePanels();
    address({ panels: [ONE], shown: ONE.key });

    await panels.open(TWO.kind, TWO.argument, { replaceArgument: ONE.argument });

    const search = withSidePanels('view=chat', { panels: [TWO], shown: TWO.key });
    expect(goto).toHaveBeenCalledExactlyOnceWith(`/?${search.toString()}`, { replace: true, reset: false });
    expect(panels.focusRequest).toBe(TWO.key);
  });

  it('holds no panels outside the main view', () => {
    const panels = new SidePanels();
    address({ panels: [ONE], shown: ONE.key });
    page.route.id = '/settings';

    expect(panels.state).toEqual({ panels: [], shown: null });
  });
});
