// SQLite's values in the Databases view (plugins/workers/docs/
// databases.md, the view): a cell shows a value on one line, cut, with NULL
// and binary data worded; the row dialog writes whole numbers as decimal
// text within SQLite's range and refuses text that is no number; a page
// copies as CSV.
import { describe, expect, it } from 'vitest';

import type { ResultColumn, SqlValue } from '../../../ui/generated/bindings.js';
import {
  blobBytes,
  CELL_CHARS,
  cellText,
  fieldOf,
  fieldProblem,
  fullText,
  isWorded,
  pageCsv,
  valueOf,
} from '../../../ui/utils/databases-values.js';

describe('cellText', () => {
  it('shows integers as their decimal text, beyond what JavaScript numbers hold', () => {
    expect(cellText({ type: 'integer', value: '9223372036854775807' })).toBe('9223372036854775807');
    expect(cellText({ type: 'real', value: 1.5 })).toBe('1.5');
  });

  it('shows text on one line, cut to its characters', () => {
    const long = 'é'.repeat(CELL_CHARS + 10);

    expect(cellText({ type: 'text', value: 'two\nlines' })).toBe('two lines');
    expect(cellText({ type: 'text', value: long })).toBe(`${'é'.repeat(CELL_CHARS)}…`);
  });

  it('words NULL, binary data and infinity from the catalog, apart from content', () => {
    const worded: SqlValue[] = [
      { type: 'null' },
      { base64: 'AP8=', type: 'blob' },
      { negative: true, type: 'infinity' },
    ];

    expect(worded.every((value) => isWorded(value))).toBe(true);
    expect(isWorded({ type: 'text', value: 'NULL' })).toBe(false);
    expect(blobBytes('AP8=')).toBe(2);
    expect(blobBytes('AAAA')).toBe(3);
  });
});

describe('fullText', () => {
  it('shows text whole, line breaks and all, and words the values that a cell words', () => {
    const long = 'x'.repeat(CELL_CHARS * 2);

    expect(fullText({ type: 'text', value: `one\n${long}` })).toBe(`one\n${long}`);
    expect(fullText({ type: 'null' })).toBe(cellText({ type: 'null' }));
  });
});

describe('the fields of the row dialog', () => {
  it('start unchanged with the value as text', () => {
    expect(fieldOf({ type: 'integer', value: '12' })).toEqual({ kind: 'unchanged', text: '12' });
    expect(fieldOf({ type: 'null' })).toEqual({ kind: 'unchanged', text: '' });
  });

  it('write whole numbers as decimal text within the 64-bit range', () => {
    expect(valueOf({ kind: 'integer', text: ' -42 ' })).toEqual({ type: 'integer', value: '-42' });
    expect(valueOf({ kind: 'integer', text: '9223372036854775808' })).toBeNull();
    expect(fieldProblem({ kind: 'integer', text: '9223372036854775808' })).not.toBeNull();
    expect(fieldProblem({ kind: 'integer', text: '1.5' })).not.toBeNull();
  });

  it('write numbers, text and NULL, and nothing for an unchanged or default column', () => {
    expect(valueOf({ kind: 'real', text: '2.5' })).toEqual({ type: 'real', value: 2.5 });
    expect(fieldProblem({ kind: 'real', text: 'two' })).not.toBeNull();
    expect(valueOf({ kind: 'text', text: '' })).toEqual({ type: 'text', value: '' });
    expect(valueOf({ kind: 'null', text: 'ignored' })).toEqual({ type: 'null' });
    expect(valueOf({ kind: 'unchanged', text: '12' })).toBeNull();
    expect(valueOf({ kind: 'default', text: '' })).toBeNull();
  });
});

describe('pageCsv', () => {
  it('writes a line of names and a line for each row, quoting what needs it', () => {
    const columns: ResultColumn[] = [
      { declaredType: 'TEXT', name: 'name' },
      { declaredType: null, name: 'note' },
    ];
    const rows: SqlValue[][] = [
      [{ type: 'text', value: 'bolt, small' }, { type: 'null' }],
      [
        { type: 'text', value: 'say "hi"' },
        { type: 'integer', value: '3' },
      ],
    ];

    expect(pageCsv(columns, rows)).toBe('name,note\r\n"bolt, small",\r\n"say ""hi""",3');
  });
});
