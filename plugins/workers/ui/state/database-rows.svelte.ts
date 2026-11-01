// One opening of the Data tab's row dialog (plugins/workers/docs/
// databases.md, the view): the row that it edits, or none for a new row,
// with a field for each of the page's columns. An edit starts with every
// column unchanged, and an insertion with every column taking its default.
import type { PageRow, ResultColumn } from '../generated/bindings.js';
import type { DatabasesFailure } from '../types/databases.js';
import { fieldOf, type ValueField } from '../utils/databases-values.js';

/** One opening of the row dialog. */
export class RowEditRequest {
  readonly columns: readonly ResultColumn[];
  /** Why the latest attempt failed. */
  failure: DatabasesFailure | null = $state(null);
  fields: ValueField[] = $state([]);
  /** The row's number on the page, from 1, or `null` for a new row. */
  readonly number: number | null;
  readonly row: PageRow | null;
  saving = $state(false);

  constructor(columns: readonly ResultColumn[], row: PageRow | null, number: number | null) {
    this.columns = columns;
    this.row = row;
    this.number = number;
    this.fields =
      row === null ? columns.map((): ValueField => ({ kind: 'default', text: '' })) : row.values.map(fieldOf);
  }
}
