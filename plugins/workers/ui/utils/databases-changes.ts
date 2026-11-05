// The owner's changes from the Data tab (plugins/workers/docs/databases.md,
// the view): an edit of a row, an insertion and a deletion of rows, each
// naming the rows by their keys and the values that the page showed, so the
// core refuses a change of a row that changed since (ADR-0056). Each change
// gets an ID of its own, which the core's journal keeps, so a request that
// arrives twice runs once.
import type { Change, ColumnValue, OwnerChange, PageRow, ResultColumn, RowMatch } from '../generated/bindings.js';

import { type ValueField, valueOf } from './databases-values.js';

/** A change with a new ID for the core's journal. */
export function ownerChange(change: Change): OwnerChange {
  return { change, id: crypto.randomUUID() };
}

/** A row as the page showed it: its key, and every column's value, which the row must still have. */
export function rowMatch(columns: readonly ResultColumn[], row: PageRow): RowMatch {
  const expected: ColumnValue[] = columns.map((column, index) => ({
    column: column.name,
    value: row.values[index] ?? { type: 'null' },
  }));
  return { expected, key: row.key };
}

/** The columns and values that the fields write, leaving out those that write none. */
function written(columns: readonly ResultColumn[], fields: readonly ValueField[]): ColumnValue[] {
  const values: ColumnValue[] = [];
  columns.forEach((column, index) => {
    const value = valueOf(fields[index]);
    if (value !== null) {
      values.push({ column: column.name, value });
    }
  });
  return values;
}

/** The edit of a row by its fields, or `null` when every field leaves its column unchanged. */
export function updateChange(
  table: string,
  columns: readonly ResultColumn[],
  row: PageRow,
  fields: readonly ValueField[],
): Change | null {
  const set = written(columns, fields);
  if (set.length === 0) {
    return null;
  }
  return { kind: 'update_row', row: rowMatch(columns, row), set, table };
}

/** The insertion of a row by its fields; the columns that the fields leave out take their defaults. */
export function insertChange(table: string, columns: readonly ResultColumn[], fields: readonly ValueField[]): Change {
  return { kind: 'insert_row', table, values: written(columns, fields) };
}

/** The deletion of rows as the page showed them, all of them or none. */
export function deleteChange(table: string, columns: readonly ResultColumn[], rows: readonly PageRow[]): Change {
  return { kind: 'delete_rows', rows: rows.map((row) => rowMatch(columns, row)), table };
}
