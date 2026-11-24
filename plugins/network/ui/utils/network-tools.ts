// Tool card summaries and sanitized card data parsers for the four Network
// Client tools (ADR-0039; ADR-0075).
import type { ToolKindCall } from '@mannie/plugin-sdk/slots';

import { m } from '../generated/messages.js';
import type { SendCardDetails } from '../types/network.js';

/** Tool name for listing enabled collections. */
export const LIST_COLLECTIONS_TOOL = 'network_list_collections';

/** Tool name for listing saved requests in a collection. */
export const LIST_REQUESTS_TOOL = 'network_list_requests';

/** Tool name for reading one saved request's redacted definition. */
export const GET_REQUEST_TOOL = 'network_get_request';

/** Tool name for executing one saved request under approval. */
export const SEND_REQUEST_TOOL = 'network_send_request';

type Fields = Readonly<Record<string, unknown>> | null;

function textOf(fields: Fields, key: string): string | null {
  const value = fields?.[key];
  if (typeof value !== 'string') {
    return null;
  }
  const trimmed = value.trim();
  return trimmed === '' ? null : trimmed;
}

function numberOf(fields: Fields, key: string): number | null {
  const value = fields?.[key];
  if (typeof value !== 'number') {
    return null;
  }
  return Number.isFinite(value) ? value : null;
}

function boolOf(fields: Fields, key: string): boolean {
  return fields?.[key] === true;
}

/** Summary line for `network_list_collections`. */
export function listCollectionsSummary(): string {
  return m.network_tool_list_collections();
}

/** Summary line for `network_list_requests`. */
export function listRequestsSummary(call: ToolKindCall): string {
  const collection = textOf(call.arguments, 'collection') ?? textOf(call.arguments, 'collection_id');
  if (collection === null) {
    return m.network_tool_list_requests();
  }
  return m.network_tool_list_requests_in({ collection });
}

/** Summary line for `network_get_request`. */
export function getRequestSummary(call: ToolKindCall): string {
  const method = textOf(call.data, 'method');
  const name =
    textOf(call.data, 'requestName') ?? textOf(call.arguments, 'request') ?? textOf(call.arguments, 'request_id');
  const hasMethod = method !== null;
  const hasName = name !== null;

  if (hasMethod && hasName) {
    return m.network_tool_get_request_named({ method, name });
  }
  return m.network_tool_get_request();
}

/** Summary line for `network_send_request`. */
export function sendRequestSummary(call: ToolKindCall): string {
  const method = textOf(call.data, 'method');
  const name = textOf(call.data, 'requestName');
  const collection = textOf(call.data, 'collectionName');
  const status = numberOf(call.data, 'status');
  const hasMethod = method !== null;
  const hasName = name !== null;
  const hasStatus = status !== null;
  const hasCollection = collection !== null;

  if (hasMethod && hasName && hasStatus) {
    return m.network_tool_send_request_status({ method, name, status: String(status) });
  }
  if (hasMethod && hasName && hasCollection) {
    return m.network_tool_send_request_named({ collection, method, name });
  }
  return m.network_tool_send_request();
}

/** Extracts sanitized, non-secret details from a `network_send_request` tool card. */
export function readSendCardDetails(data: Fields): SendCardDetails | null {
  if (data === null) {
    return null;
  }
  const url = textOf(data, 'url');
  const status = numberOf(data, 'status');
  const isEmpty = url === null && status === null;

  if (isEmpty) {
    return null;
  }
  return {
    bytes: numberOf(data, 'bytes'),
    collectionName: textOf(data, 'collectionName'),
    durationMs: numberOf(data, 'durationMs'),
    method: textOf(data, 'method'),
    protocol: textOf(data, 'protocol'),
    requestName: textOf(data, 'requestName'),
    status,
    statusText: textOf(data, 'statusText'),
    truncated: boolOf(data, 'truncated'),
    url,
  };
}
