// The lines of the cards of Databases' tools (ADR-0039; plugins/workers/
// docs/databases.md, agents): what a call did, from the data that the
// plugin returned for its card, or from its arguments while it runs or when
// it failed. A database's name is content and stays as it is; every word
// around it comes from the catalog (ADR-0019).
import type { ToolKindCall } from '@mannie/plugin-sdk/slots';

import { m } from '../generated/messages.js';

/** The feature's tools, by their names in the plugin's tool specs. */
export const DESCRIBE_DATABASE_TOOL = 'describe_database';
export const QUERY_DATABASE_TOOL = 'query_database';
export const CHANGE_DATABASE_TOOL = 'change_database';

type Fields = Readonly<Record<string, unknown>> | null;

/** A field that holds text that is not empty, or `null`. */
function textOf(fields: Fields, key: string): string | null {
  const value = fields?.[key];
  if (typeof value !== 'string') {
    return null;
  }
  return value === '' ? null : value;
}

/** A field that holds a whole number that is not negative, or `null`. */
function countOf(fields: Fields, key: string): number | null {
  const value = fields?.[key];
  const isCount = typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
  return isCount ? value : null;
}

/** The sum of a field that holds whole numbers, such as the rows that each statement changed, or `null`. */
function sumOf(fields: Fields, key: string): number | null {
  const values = fields?.[key];
  if (!Array.isArray(values)) {
    return null;
  }
  let sum = 0;
  for (const value of values) {
    const isCount = typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
    if (!isCount) {
      return null;
    }
    sum += value;
  }
  return sum;
}

/** The database that a call names: the one that its card names, or its argument. */
function databaseOf(call: ToolKindCall): string | null {
  return textOf(call.data, 'database') ?? textOf(call.arguments, 'database');
}

/** The line of a call of `describe_database`, with its tables once it ended. */
export function describeSummary(call: ToolKindCall): string | null {
  const database = databaseOf(call);
  if (database === null) {
    return null;
  }
  const tables = countOf(call.data, 'tables');
  return tables === null
    ? m.workers_databases_tool_describe({ database })
    : m.workers_databases_tool_describe_done({ count: tables, database });
}

/** The line of a call of `query_database`, with its rows once it ended. */
export function querySummary(call: ToolKindCall): string | null {
  const database = databaseOf(call);
  if (database === null) {
    return null;
  }
  const rows = countOf(call.data, 'rows');
  return rows === null
    ? m.workers_databases_tool_query({ database })
    : m.workers_databases_tool_query_done({ count: rows, database });
}

/** The line of a call of `change_database`, with the rows that it changed once it ended. */
export function changeSummary(call: ToolKindCall): string | null {
  const database = databaseOf(call);
  if (database === null) {
    return null;
  }
  const changes = sumOf(call.data, 'changes');
  return changes === null
    ? m.workers_databases_tool_change({ database })
    : m.workers_databases_tool_change_done({ count: changes, database });
}
