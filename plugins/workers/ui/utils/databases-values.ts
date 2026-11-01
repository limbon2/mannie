// SQLite's values in the Databases view (plugins/workers/docs/databases.md,
// the view): how a cell shows a value, how the row dialog edits one, and a
// page as CSV. Integers stay decimal text from end to end, since they may
// be larger than JavaScript's numbers hold. A value is content: text shows
// as it is, cut, and only NULL and binary data are worded from the catalog.
import { formatBytes } from '@mannie/plugin-sdk/format';

import type { ResultColumn, SqlValue } from '../generated/bindings.js';
import { m } from '../generated/messages.js';

/** The most characters of a text that a cell shows; the row dialog shows it whole. */
export const CELL_CHARS = 200;

/** The largest and smallest integers that SQLite stores. */
const MAX_INTEGER = 9_223_372_036_854_775_807n;
const MIN_INTEGER = -9_223_372_036_854_775_808n;

/** How many bytes a blob's standard base64 holds. */
export function blobBytes(base64: string): number {
  let padding = 0;
  if (base64.endsWith('==')) {
    padding = 2;
  } else if (base64.endsWith('=')) {
    padding = 1;
  }
  return Math.max(0, (base64.length / 4) * 3 - padding);
}

/** The characters of a text as its reader sees them, emoji and combined letters whole. */
const graphemes = new Intl.Segmenter(undefined, { granularity: 'grapheme' });

/** A text cut to `max` characters, with an ellipsis in place of the rest. */
function cut(text: string, max: number): string {
  let count = 0;
  for (const { index } of graphemes.segment(text)) {
    if (count === max) {
      return `${text.slice(0, index)}…`;
    }
    count += 1;
  }
  return text;
}

/** The kinds of value that a cell words from the catalog. */
const WORDED: ReadonlySet<SqlValue['type']> = new Set(['blob', 'infinity', 'null']);

/** A value as a cell shows it, on one line and cut. */
export function cellText(value: SqlValue): string {
  switch (value.type) {
    case 'blob': {
      return m.workers_databases_value_blob({ size: formatBytes(blobBytes(value.base64)) });
    }
    case 'infinity': {
      return value.negative ? m.workers_databases_value_negative_infinity() : m.workers_databases_value_infinity();
    }
    case 'integer': {
      return value.value;
    }
    case 'null': {
      return m.workers_databases_value_null();
    }
    case 'real': {
      return value.value === null ? m.workers_databases_value_null() : String(value.value);
    }
    case 'text': {
      return cut(value.value.replaceAll(/\s+/gu, ' '), CELL_CHARS);
    }
  }
}

/** A value whole, as the cell's dialog shows it: text and numbers as they are, the others worded. */
export function fullText(value: SqlValue): string {
  switch (value.type) {
    case 'integer':
    case 'text': {
      return value.value;
    }
    case 'real': {
      return value.value === null ? m.workers_databases_value_null() : String(value.value);
    }
    case 'blob':
    case 'infinity':
    case 'null': {
      return cellText(value);
    }
  }
}

/** Whether a value shows from the catalog, such as NULL, rather than as the database's content. */
export function isWorded(value: SqlValue): boolean {
  return WORDED.has(value.type);
}

/** The kinds of value that the row dialog writes. */
export type ValueKind = 'default' | 'integer' | 'null' | 'real' | 'text' | 'unchanged';

/** A field of the row dialog: the kind of value and its text. */
export interface ValueField {
  kind: ValueKind;
  text: string;
}

/** The field of a value that a row has, which starts unchanged with the value's text. */
export function fieldOf(value: SqlValue): ValueField {
  switch (value.type) {
    case 'integer':
    case 'text': {
      return { kind: 'unchanged', text: value.value };
    }
    case 'real': {
      return { kind: 'unchanged', text: value.value === null ? '' : String(value.value) };
    }
    case 'blob':
    case 'infinity':
    case 'null': {
      return { kind: 'unchanged', text: '' };
    }
  }
}

/** Why a field's text is not a value of its kind, or `null` when it is. */
export function fieldProblem(field: ValueField): string | null {
  switch (field.kind) {
    case 'integer': {
      return integerOf(field.text) === null ? m.workers_databases_row_invalid_integer() : null;
    }
    case 'real': {
      return realOf(field.text) === null ? m.workers_databases_row_invalid_real() : null;
    }
    case 'default':
    case 'null':
    case 'text':
    case 'unchanged': {
      return null;
    }
  }
}

/** An integer's decimal text, trimmed, or `null` for text that is no 64-bit integer. */
function integerOf(text: string): string | null {
  const trimmed = text.trim();
  if (!/^-?\d+$/u.test(trimmed)) {
    return null;
  }
  const integer = BigInt(trimmed);
  const fits = integer >= MIN_INTEGER && integer <= MAX_INTEGER;
  return fits ? integer.toString() : null;
}

/** A finite number of the text, or `null` for text that is none. */
function realOf(text: string): number | null {
  const trimmed = text.trim();
  if (trimmed === '') {
    return null;
  }
  const number = Number(trimmed);
  return Number.isFinite(number) ? number : null;
}

/**
 * The value that a field writes, or `null` for a field that writes none:
 * one left unchanged, one that takes the column's default, or one whose
 * text is no value of its kind.
 */
export function valueOf(field: ValueField): SqlValue | null {
  switch (field.kind) {
    case 'default':
    case 'unchanged': {
      return null;
    }
    case 'integer': {
      const integer = integerOf(field.text);
      return integer === null ? null : { type: 'integer', value: integer };
    }
    case 'null': {
      return { type: 'null' };
    }
    case 'real': {
      const real = realOf(field.text);
      return real === null ? null : { type: 'real', value: real };
    }
    case 'text': {
      return { type: 'text', value: field.text };
    }
  }
}

/** A value as a CSV field writes it: NULL empty, a blob as its base64. */
function csvValue(value: SqlValue): string {
  switch (value.type) {
    case 'blob': {
      return value.base64;
    }
    case 'infinity': {
      return value.negative ? '-Inf' : 'Inf';
    }
    case 'integer':
    case 'text': {
      return value.value;
    }
    case 'null': {
      return '';
    }
    case 'real': {
      return value.value === null ? '' : String(value.value);
    }
  }
}

/** A CSV field, quoted when it holds a comma, a quote or a line break (RFC 4180). */
function csvField(text: string): string {
  return /[",\r\n]/u.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
}

/** A page as CSV: a line of the columns' names, then a line for each row. */
export function pageCsv(columns: readonly ResultColumn[], rows: readonly (readonly SqlValue[])[]): string {
  const header = columns.map((column) => csvField(column.name)).join(',');
  const lines = rows.map((row) => row.map((value) => csvField(csvValue(value))).join(','));
  return [header, ...lines].join('\r\n');
}
