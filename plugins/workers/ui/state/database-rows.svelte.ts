// One opening of the Data tab's row dialog (plugins/workers/docs/
// databases.md, the view): the row that it edits, or none for a new row,
// with a field for each of the page's columns. An edit starts with each
// column's value, which it leaves unchanged until the owner changes it, and
// an insertion with each column taking its default. A field writes the kind
// of value that its column's declared type gives; a column without one
// writes the kind of its value until the owner chooses another. Each
// change can be undone to the field's start.
import type { PageRow, ResultColumn } from '../generated/bindings.js';
import type { DatabasesFailure } from '../types/databases.js';
import { columnKind, fieldOf, kindOfValue, type TypedKind, type ValueField } from '../utils/databases-values.js';

/** One opening of the row dialog. */
export class RowEditRequest {
  readonly columns: readonly ResultColumn[];
  /** Why the latest attempt failed. */
  failure: DatabasesFailure | null = $state(null);
  fields: ValueField[] = $state([]);
  /** The kind of value that each field's typed text writes. */
  kinds: TypedKind[] = $state([]);
  /** The row's number on the page, from 1, or `null` for a new row. */
  readonly number: number | null;
  readonly row: PageRow | null;
  saving = $state(false);
  /** Whether each column's declared type gives its kind, so the owner does not choose it. */
  readonly typed: readonly boolean[];
  /** Each field as it started, which an undo restores. */
  readonly #initial: readonly ValueField[];

  constructor(columns: readonly ResultColumn[], row: PageRow | null, number: number | null) {
    this.columns = columns;
    this.row = row;
    this.number = number;
    this.#initial =
      row === null ? columns.map((): ValueField => ({ kind: 'default', text: '' })) : row.values.map(fieldOf);
    this.fields = this.#initial.map((field) => ({ ...field }));
    const declared = columns.map((column) => columnKind(column.declaredType));
    this.typed = declared.map((kind) => kind !== null);
    this.kinds = declared.map((kind, index) => kind ?? kindOfValue(row?.values[index]));
  }

  /** Whether a field differs from its start. */
  changed(index: number): boolean {
    const field = this.fields[index];
    const initial = this.#initial[index];
    const otherKind = field.kind !== initial.kind;
    const otherText = field.text !== initial.text;
    return otherKind || otherText;
  }

  /** Writes the owner's text into a field; the start's text leaves the field as it started. */
  edit(index: number, text: string): void {
    const initial = this.#initial[index];
    const backAtStart = text === initial.text && this.fields[index].kind !== 'null';
    this.fields[index] = backAtStart ? { ...initial } : { kind: this.kinds[index], text };
  }

  /** Whether a field shows NULL: one set to NULL, or a row's NULL left unchanged. */
  isNull(index: number): boolean {
    const field = this.fields[index];
    const wasNull = this.row?.values[index]?.type === 'null';
    const unchangedNull = field.kind === 'unchanged' && wasNull;
    const setToNull = field.kind === 'null';
    return setToNull || unchangedNull;
  }

  /** Chooses the kind of value that a column without a type writes; a changed field writes it at once. */
  setKind(index: number, kind: TypedKind): void {
    this.kinds[index] = kind;
    const field = this.fields[index];
    const writesText = this.changed(index) && field.kind !== 'null';
    if (writesText) {
      this.fields[index] = { kind, text: field.text };
    }
  }

  /** Sets a field to NULL. */
  setNull(index: number): void {
    this.fields[index] = { kind: 'null', text: '' };
  }

  /** Puts a field back as it started. */
  undo(index: number): void {
    this.fields[index] = { ...this.#initial[index] };
  }
}
