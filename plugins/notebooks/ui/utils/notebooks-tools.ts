// The lines of the cards of the plugin's tools (ADR-0039; ADR-0052): what a
// call did, from the data that the plugin returned for its card, or from its
// arguments while it runs or when it failed. Names, queries and headings are
// content and stay as they are; every word around them comes from the
// catalog (ADR-0019).
import type { ToolKindCall } from '@mannie/plugin-sdk/slots';

import { m } from '../generated/messages.js';
import { notebookLibrary } from '../state/notebooks.svelte.js';

/** The plugin's tools, by their names in the plugin's tool specs. */
export const LIST_SOURCES_TOOL = 'list_notebook_sources';
export const SEARCH_TOOL = 'search_notebooks';
export const READ_SOURCE_TOOL = 'read_notebook_source';

type Fields = Readonly<Record<string, unknown>> | null;

/** A field that holds text that is not empty, or `null`. */
function textOf(fields: Fields, key: string): string | null {
  const value = fields?.[key];
  if (typeof value !== 'string') {
    return null;
  }
  return value === '' ? null : value;
}

/** A field that holds a finite number, or `null`. */
function numberOf(fields: Fields, key: string): number | null {
  const value = fields?.[key];
  if (typeof value !== 'number') {
    return null;
  }
  return Number.isFinite(value) ? value : null;
}

/** The first and last pages that a read gave, or `null` when it did not read pages. */
function pagesOf(fields: Fields): { first: number; last: number } | null {
  const first = numberOf(fields, 'firstPage');
  const last = numberOf(fields, 'lastPage');
  if (first === null) {
    return null;
  }
  return last === null ? null : { first, last };
}

/** The line of a call of `list_notebook_sources`. */
export function listSummary(): string {
  return m.notebooks_tool_list();
}

/** The line of a call of `search_notebooks`: its query, and how many passages it found once it ended. */
export function searchSummary(call: ToolKindCall): string | null {
  const query = textOf(call.data, 'query') ?? textOf(call.arguments, 'query');
  if (query === null) {
    return null;
  }
  const found = numberOf(call.data, 'found');
  return found === null ? m.notebooks_tool_search({ query }) : m.notebooks_tool_search_found({ count: found, query });
}

/** The line of a call of `read_notebook_source`: the source, with the pages or the heading that it read. */
export function readSummary(call: ToolKindCall): string | null {
  const sourceId = textOf(call.arguments, 'source');
  const named = sourceId === null ? undefined : notebookLibrary.findSource(sourceId)?.name;
  const source = textOf(call.data, 'source') ?? named ?? null;
  if (source === null) {
    return null;
  }
  const pages = pagesOf(call.data);
  if (pages !== null) {
    const { first, last } = pages;
    return first === last
      ? m.notebooks_tool_read_page({ page: first, source })
      : m.notebooks_tool_read_pages({ first, last, source });
  }
  const heading = textOf(call.data, 'heading');
  return heading === null ? m.notebooks_tool_read({ source }) : m.notebooks_tool_read_heading({ heading, source });
}
