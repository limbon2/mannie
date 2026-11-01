// The owner's changes from the Data tab (plugins/workers/docs/databases.md,
// the view): an edit writes only the columns that the owner changed and
// names its row by its key and every value that the page showed, an
// insertion leaves the defaults out, a deletion names each row the same
// way, and every change gets an ID of its own.
import { describe, expect, it } from 'vitest';

import type { PageRow, ResultColumn } from '../../../ui/generated/bindings.js';
import { deleteChange, insertChange, ownerChange, updateChange } from '../../../ui/utils/databases-changes.js';

const COLUMNS: ResultColumn[] = [
  { declaredType: 'TEXT', name: 'name' },
  { declaredType: 'INTEGER', name: 'stock' },
];

const BOLT: PageRow = {
  key: [{ type: 'integer', value: '1' }],
  values: [
    { type: 'text', value: 'bolt' },
    { type: 'integer', value: '12' },
  ],
};

const EXPECTED = [
  { column: 'name', value: { type: 'text', value: 'bolt' } },
  { column: 'stock', value: { type: 'integer', value: '12' } },
];

describe('updateChange', () => {
  it('writes only the changed columns and expects every value that the page showed', () => {
    const change = updateChange('parts', COLUMNS, BOLT, [
      { kind: 'unchanged', text: 'bolt' },
      { kind: 'integer', text: '11' },
    ]);

    expect(change).toEqual({
      kind: 'update_row',
      row: { expected: EXPECTED, key: [{ type: 'integer', value: '1' }] },
      set: [{ column: 'stock', value: { type: 'integer', value: '11' } }],
      table: 'parts',
    });
  });

  it('is nothing when every column stays unchanged', () => {
    const fields = [
      { kind: 'unchanged' as const, text: 'bolt' },
      { kind: 'unchanged' as const, text: '12' },
    ];

    expect(updateChange('parts', COLUMNS, BOLT, fields)).toBeNull();
  });
});

describe('insertChange and deleteChange', () => {
  it('leave the columns that keep their default out of an insertion', () => {
    const change = insertChange('parts', COLUMNS, [
      { kind: 'text', text: 'gear' },
      { kind: 'default', text: '' },
    ]);

    expect(change).toEqual({
      kind: 'insert_row',
      table: 'parts',
      values: [{ column: 'name', value: { type: 'text', value: 'gear' } }],
    });
  });

  it('name each deleted row by its key and the values that the page showed', () => {
    expect(deleteChange('parts', COLUMNS, [BOLT])).toEqual({
      kind: 'delete_rows',
      rows: [{ expected: EXPECTED, key: [{ type: 'integer', value: '1' }] }],
      table: 'parts',
    });
  });
});

describe('ownerChange', () => {
  it('gives each change an ID of its own in the form that the core takes', () => {
    const change = deleteChange('parts', COLUMNS, [BOLT]);
    const first = ownerChange(change);
    const second = ownerChange(change);

    expect(first.id).toMatch(/^[\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12}$/u);
    expect(first.id).not.toBe(second.id);
    expect(first.change).toBe(change);
  });
});
