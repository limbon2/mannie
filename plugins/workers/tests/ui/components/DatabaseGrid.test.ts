import { SvelteSet } from 'svelte/reactivity';
// Rows of a database in a table (plugins/workers/docs/databases.md, the
// view), rendered on the server side: the columns' names and the values as
// content, NULL and binary data worded and muted, and the selection and the
// edit action only where the Data tab asks for them.
import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';

import DatabaseGrid from '../../../ui/components/DatabaseGrid.svelte';
import type { ResultColumn, SqlValue } from '../../../ui/generated/bindings.js';

const COLUMNS: ResultColumn[] = [
  { declaredType: 'TEXT', name: 'name' },
  { declaredType: 'INTEGER', name: 'stock' },
];

const ROWS: SqlValue[][] = [
  [
    { type: 'text', value: 'bolt' },
    { type: 'integer', value: '12' },
  ],
  [{ type: 'null' }, { base64: 'AP8=', type: 'blob' }],
];

describe('DatabaseGrid', () => {
  it('shows the columns and the values, with NULL and binary data worded and muted', () => {
    const body = render(DatabaseGrid, { props: { columns: COLUMNS, rows: ROWS } }).body;

    expect(body).toContain('name');
    expect(body).toContain('bolt');
    expect(body).toContain('12');
    expect(body).toContain('NULL');
    expect(body).toContain('Binary data, 2');
    expect(body).toContain('italic');
    expect(body).not.toContain('role="checkbox"');
  });

  it('offers the selection and the edit action only when asked', () => {
    const body = render(DatabaseGrid, {
      props: { columns: COLUMNS, onedit: () => undefined, rows: ROWS, selected: new SvelteSet<number>([0]) },
    }).body;

    expect(body).toContain('role="checkbox"');
    expect(body).toContain('Select every row of the page');
    expect(body).toContain('Edit row 1');
    expect(body).toContain('data-state="selected"');
  });
});
