// Floating layers of the TypeScript plugin SDK (ADR-0063, decision 5): a
// plugin learns which of the shell's dialogs, menus, selects, popovers and
// tooltips are open and where, from the bits-ui markers.
import { describe, expect, it } from 'vitest';

import {
  floatingLayers,
  type LayerElement,
  type LayerRect,
  type LayerRoot,
} from '../../../../../../apps/desktop/src/lib/sdk/surface.js';

function element(attributes: Record<string, string>, rect: LayerRect): LayerElement {
  return {
    getAttribute: (name) => attributes[name] ?? null,
    getBoundingClientRect: () => rect,
  };
}

function root(elements: LayerElement[]): LayerRoot & { selectors: string[] } {
  const selectors: string[] = [];
  return {
    querySelectorAll(selector: string) {
      selectors.push(selector);
      return elements;
    },
    selectors,
  };
}

const RECT = { height: 120, width: 200, x: 10, y: 20 };

describe('floatingLayers', () => {
  it('reads the kind and place of each open layer', () => {
    const shell = root([
      element({ 'data-dropdown-menu-content': '', 'data-state': 'open' }, RECT),
      element({ 'data-tooltip-content': '' }, { height: 20, width: 80, x: 300, y: 40 }),
      element({ 'data-alert-dialog-content': '' }, RECT),
      element({ 'data-select-content': '' }, RECT),
      element({ 'data-popover-content': '' }, RECT),
    ]);

    expect(floatingLayers(shell).map((layer) => layer.kind)).toEqual([
      'menu',
      'tooltip',
      'dialog',
      'select',
      'popover',
    ]);
    expect(floatingLayers(shell)[1]?.rect).toEqual({ height: 20, width: 80, x: 300, y: 40 });
    expect(shell.selectors[0]).toContain('[data-context-menu-content]');
  });

  it('leaves out closing layers and empty ones, but never a dialog', () => {
    const empty = { height: 0, width: 0, x: 0, y: 0 };
    const shell = root([
      element({ 'data-menu-content': '', 'data-state': 'closed' }, RECT),
      element({ 'data-tooltip-content': '' }, empty),
      element({ 'data-dialog-content': '' }, empty),
    ]);

    expect(floatingLayers(shell)).toEqual([{ kind: 'dialog', rect: empty }]);
  });
});
