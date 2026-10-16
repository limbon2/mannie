// The help of a form field (docs/engineering/standards.md, UI and visible
// text), rendered on the server side: a button named after the field, and
// the help text in the page under the ID that the field's control names in
// `aria-describedby`, which screen readers read while the tooltip is closed.
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';

import FieldHelp from '../../../../../../../apps/desktop/src/lib/components/app/FieldHelp.svelte';

describe('FieldHelp', () => {
  it('names its button after the field and keeps the text in the page under its ID', () => {
    const { body } = render(FieldHelp, {
      props: { field: 'Address', id: 'pair-worker-address-help', text: 'The name of the host.' },
    });

    expect(body).toMatch(/<button[^>]*aria-label="Help: Address"/u);
    expect(body).toMatch(/<button[^>]*type="button"/u);
    expect(body).toMatch(/<span[^>]*id="pair-worker-address-help"[^>]*>The name of the host\.<\/span>/u);
  });
});
