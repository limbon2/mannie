// @ts-check
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';

import { starsAfterHas } from '../../scripts/check-css.mjs';

describe('starsAfterHas', () => {
  it("finds Tailwind's group-has variants, a root that :has() a descendant, and a :has() with nested parentheses", () => {
    const css = [
      '.group-has-\\[\\>svg\\]\\/alert\\:col-start-2:is(:where(.group\\/alert):has(>svg) *){grid-column-start:2}',
      ':root:has([data-pane-resizer][data-active=pointer]) *{cursor:ew-resize!important}',
      '.a:is(:where(.group\\/field):has(:where([data-orientation=horizontal])) *){text-wrap:balance}',
      '.b:has(>[data-x])>*{margin:0}',
    ].join('');

    assert.deepEqual(starsAfterHas(css), [
      '.group-has-\\[\\>svg\\]\\/alert\\:col-start-2:is(:where(.group\\/alert):has(>svg) *)',
      ':root:has([data-pane-resizer][data-active=pointer]) *',
      '.a:is(:where(.group\\/field):has(:where([data-orientation=horizontal])) *)',
      '.b:has(>[data-x])>*',
    ]);
  });

  it('leaves a :has() on the element it styles, or followed by a named element, and :has() in an at-rule', () => {
    const css = [
      '.has-data-checked\\:border-primary:has([data-checked]){border-color:red}',
      '[data-slot=alert]:has(>svg)>[data-slot=alert-title]{grid-column-start:2}',
      '.c:has(>[data-slot=field-content]) [role=radio]{margin-top:1px}',
      '@supports selector(:has(*)){.d{color:red}}',
      '* {outline-color:red}',
      '/* :root:has(.x) * */.e{color:red}',
    ].join('\n');

    assert.deepEqual(starsAfterHas(css), []);
  });

  it('finds the selector that puts * after a :has() in a list, and only that one', () => {
    const css = '.f:hover,.g:is(:where(.group\\/item):has([data-slot=item-description]) *),.h{color:red}';

    assert.deepEqual(starsAfterHas(css), ['.g:is(:where(.group\\/item):has([data-slot=item-description]) *)']);
  });
});
