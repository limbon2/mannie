// One opening of the Data tab's row dialog (plugins/workers/docs/
// databases.md, the view): a field starts with the row's value and writes
// nothing until the owner changes it, writes the kind that its column's
// type gives, can be set to NULL and undone to its start, and a new row's
// fields take their defaults until the owner writes a value.
import { describe, expect, it } from 'vitest';

import type { PageRow, ResultColumn } from '../../../ui/generated/bindings.js';
import { RowEditRequest } from '../../../ui/state/database-rows.svelte.js';
import { updateChange } from '../../../ui/utils/databases-changes.js';

const COLUMNS: ResultColumn[] = [
  { declaredType: 'TEXT', name: 'name' },
  { declaredType: 'INTEGER', name: 'stock' },
  { declaredType: null, name: 'note' },
];

const ROW: PageRow = {
  key: [{ type: 'text', value: 'bolt' }],
  values: [{ type: 'text', value: 'bolt' }, { type: 'integer', value: '12' }, { type: 'null' }],
};

describe('RowEditRequest', () => {
  it('starts with the row values and writes nothing until a field changes', () => {
    const request = new RowEditRequest(COLUMNS, ROW, 1);

    expect(request.fields.map((field) => field.text)).toEqual(['bolt', '12', '']);
    expect(request.typed).toEqual([true, true, false]);
    expect(request.kinds).toEqual(['text', 'integer', 'text']);
    expect(request.isNull(2)).toBe(true);
    expect(updateChange('parts', COLUMNS, ROW, request.fields)).toBeNull();
  });

  it('writes the kind that the column type gives, and undoes a change', () => {
    const request = new RowEditRequest(COLUMNS, ROW, 1);

    request.edit(1, '11');
    expect(request.fields[1]).toEqual({ kind: 'integer', text: '11' });
    expect(request.changed(1)).toBe(true);

    request.undo(1);
    expect(request.changed(1)).toBe(false);
    expect(updateChange('parts', COLUMNS, ROW, request.fields)).toBeNull();
  });

  it('leaves a field unchanged when the owner types its value back', () => {
    const request = new RowEditRequest(COLUMNS, ROW, 1);

    request.edit(0, 'bol');
    request.edit(0, 'bolt');

    expect(request.changed(0)).toBe(false);
  });

  it('sets a field to NULL, and a value typed after it replaces the NULL', () => {
    const request = new RowEditRequest(COLUMNS, ROW, 1);

    request.setNull(0);
    expect(request.isNull(0)).toBe(true);
    expect(request.changed(0)).toBe(true);

    request.edit(0, 'bolt');
    expect(request.fields[0]).toEqual({ kind: 'text', text: 'bolt' });
  });

  it('lets a column without a type choose its kind, which a changed field writes', () => {
    const request = new RowEditRequest(COLUMNS, ROW, 1);

    request.setKind(2, 'integer');
    expect(request.changed(2)).toBe(false);
    request.edit(2, '7');
    expect(request.fields[2]).toEqual({ kind: 'integer', text: '7' });
    request.setKind(2, 'real');
    expect(request.fields[2]).toEqual({ kind: 'real', text: '7' });
  });

  it('starts a new row with every column taking its default', () => {
    const request = new RowEditRequest(COLUMNS, null, null);

    expect(request.fields.every((field) => field.kind === 'default')).toBe(true);
    request.edit(0, 'gear');
    expect(request.fields[0]).toEqual({ kind: 'text', text: 'gear' });
    request.undo(0);
    expect(request.fields[0]).toEqual({ kind: 'default', text: '' });
  });
});
