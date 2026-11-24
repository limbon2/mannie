// URL template synchronization (Notes 144, 159), variable IntelliSense (Note
// 160) and syntax language inference (Note 153) for the Network Client UI.
import type { CodeCompletionItem, CodeEditorLanguage } from '@mannie/plugin-sdk/code-editor';

import type {
  CollectionView,
  CookieView,
  KeyValueEntry,
  OpenApiWarning,
  VariableEntry,
} from '../generated/bindings.js';

/** Scope where an available `{{variable}}` is defined. */
export type VariableScope = 'collection' | 'environment' | 'global';

/** Resolved variable suggestion shown in IntelliSense popups. */
export interface VariableSuggestion {
  readonly description: string;
  readonly key: string;
  readonly scope: VariableScope;
  readonly secret: boolean;
  readonly value: string;
}

/** Translated labels used when formatting variable suggestions for CodeMirror. */
export interface VariableCompletionLabels {
  readonly collection: string;
  readonly environment: string;
  readonly global: string;
  readonly redacted: string;
}

interface SplitUrlParts {
  readonly base: string;
  readonly fragment: string | null;
  readonly query: string | null;
}

const SINGLE_BRACE_PARAM = /(?<!\{)\{([^{}/\s?&#]+)\}(?!\})/gu;

function splitUrlParts(urlTemplate: string): SplitUrlParts {
  const hashIndex = urlTemplate.indexOf('#');
  const beforeHash = hashIndex === -1 ? urlTemplate : urlTemplate.slice(0, hashIndex);
  const fragment = hashIndex === -1 ? null : urlTemplate.slice(hashIndex + 1);
  const queryIndex = beforeHash.indexOf('?');

  if (queryIndex === -1) {
    return { base: beforeHash, fragment, query: null };
  }

  return {
    base: beforeHash.slice(0, queryIndex),
    fragment,
    query: beforeHash.slice(queryIndex + 1),
  };
}

/**
 * Extracts unique `{param}` path placeholder names from a URL template,
 * ignoring `{{variable}}` interpolations, query strings and fragments.
 */
export function extractPathParamNames(urlTemplate: string): string[] {
  const { base } = splitUrlParts(urlTemplate);
  const names: string[] = [];
  const seen = new Set<string>();

  for (const match of base.matchAll(SINGLE_BRACE_PARAM)) {
    const name = match[1].trim();
    const isNewName = name !== '' && !seen.has(name);

    if (isNewName) {
      seen.add(name);
      names.push(name);
    }
  }

  return names;
}

/**
 * Derives the path parameter list one-way from the `{param}` placeholders
 * currently present in `urlTemplate`. Remembered entries preserve the owner's
 * typed value, description and secret flag if a placeholder is temporarily
 * edited and restored.
 */
export function syncPathParamsFromUrl(
  urlTemplate: string,
  existing: readonly KeyValueEntry[],
  remembered: Map<string, KeyValueEntry>,
): KeyValueEntry[] {
  for (const entry of existing) {
    const key = entry.key.trim();
    if (key !== '') {
      remembered.set(key, { ...entry, key });
    }
  }

  const names = extractPathParamNames(urlTemplate);
  return names.map((name) => {
    const prev = existing.find((entry) => entry.key.trim() === name) ?? remembered.get(name);

    if (prev !== undefined) {
      return {
        description: prev.description,
        enabled: true,
        key: name,
        secret: prev.secret,
        value: prev.value,
      };
    }

    return {
      description: '',
      enabled: true,
      key: name,
      secret: false,
      value: '',
    };
  });
}

/** Removes the query string from a URL template while keeping its path and any fragment. */
export function stripQueryFromUrl(urlTemplate: string): string {
  const { base, fragment } = splitUrlParts(urlTemplate);
  return fragment === null ? base : `${base}#${fragment}`;
}

function encodeQueryToken(raw: string): string {
  // Keep `{{variable}}` placeholders legible in the URL bar while encoding
  // delimiter characters (`&`, `=`, `#`, spaces) so round-tripping stays exact.
  return raw
    .replaceAll('%', '%25')
    .replaceAll('&', '%26')
    .replaceAll('=', '%3D')
    .replaceAll('#', '%23')
    .replaceAll(' ', '%20');
}

function decodeQueryToken(raw: string): string {
  try {
    return decodeURIComponent(raw.replaceAll('+', ' '));
  } catch {
    return raw;
  }
}

/**
 * Composes a Postman-style URL bar value from a base URL template and the
 * enabled query parameters.
 */
export function composeUrlWithQuery(urlTemplate: string, queryParams: readonly KeyValueEntry[]): string {
  const { base, fragment } = splitUrlParts(urlTemplate);
  const pairs: string[] = [];

  for (const entry of queryParams) {
    if (!entry.enabled) {
      continue;
    }

    const key = entry.key.trim();
    if (key === '') {
      continue;
    }

    const encodedKey = encodeQueryToken(key);
    const encodedValue = encodeQueryToken(entry.value);
    pairs.push(encodedValue === '' ? encodedKey : `${encodedKey}=${encodedValue}`);
  }

  const withQuery = pairs.length > 0 ? `${base}?${pairs.join('&')}` : base;
  return fragment === null ? withQuery : `${withQuery}#${fragment}`;
}

/**
 * Updates `queryParams` when the owner edits the URL bar:
 * - parses enabled key=value pairs from `?…` in the URL;
 * - preserves `secret` and `description` on matching existing enabled rows;
 * - preserves disabled rows so unchecking a query row never deletes it.
 */
export function syncQueryParamsFromUrl(urlTemplate: string, existing: readonly KeyValueEntry[]): KeyValueEntry[] {
  const { query } = splitUrlParts(urlTemplate);
  const disabledRows = existing.filter((entry) => !entry.enabled);
  const remainingEnabled = existing.filter((entry) => entry.enabled);
  const nextEnabled: KeyValueEntry[] = [];

  const hasQuery = query !== null && query !== '';

  if (hasQuery) {
    for (const part of query.split('&')) {
      if (part === '') {
        continue;
      }

      const eqIndex = part.indexOf('=');
      const rawKey = eqIndex === -1 ? part : part.slice(0, eqIndex);
      const rawValue = eqIndex === -1 ? '' : part.slice(eqIndex + 1);
      const key = decodeQueryToken(rawKey).trim();
      const value = decodeQueryToken(rawValue);

      if (key === '') {
        continue;
      }

      const matchIndex = remainingEnabled.findIndex((entry) => entry.key.trim() === key);

      if (matchIndex !== -1) {
        const matched = remainingEnabled.splice(matchIndex, 1).at(0);
        if (matched !== undefined) {
          nextEnabled.push({
            ...matched,
            enabled: true,
            key,
            value,
          });
        }
      } else {
        nextEnabled.push({
          description: '',
          enabled: true,
          key,
          secret: false,
          value,
        });
      }
    }
  }

  return [...nextEnabled, ...disabledRows];
}

/**
 * Returns all enabled variables reachable from a collection and environment in
 * resolution precedence order: environment -> collection -> global.
 */
export function availableVariables(
  collection: Pick<CollectionView, 'environments' | 'variables'> | null | undefined,
  environmentId: string | null | undefined,
  globalVariables: readonly VariableEntry[],
): VariableSuggestion[] {
  const seen = new Set<string>();
  const suggestions: VariableSuggestion[] = [];

  const collect = (entries: readonly VariableEntry[], scope: VariableScope): void => {
    for (const entry of entries) {
      if (entry.enabled === false) {
        continue;
      }
      const key = entry.key.trim();
      const isEmptyKey = key === '';

      if (isEmptyKey || seen.has(key)) {
        continue;
      }
      seen.add(key);
      suggestions.push({
        description: entry.description ?? '',
        key,
        scope,
        secret: entry.secret ?? false,
        value: entry.value,
      });
    }
  };

  if (collection != null) {
    if (environmentId != null) {
      const env = collection.environments.find((candidate) => candidate.id === environmentId);
      if (env !== undefined) {
        collect(env.variables, 'environment');
      }
    }
    collect(collection.variables, 'collection');
  }

  collect(globalVariables, 'global');
  return suggestions;
}

function labelForScope(scope: VariableScope, labels: VariableCompletionLabels): string {
  switch (scope) {
    case 'environment': {
      return labels.environment;
    }
    case 'collection': {
      return labels.collection;
    }
    case 'global': {
      return labels.global;
    }
  }
}

/** Converts variable suggestions into CodeMirror completion entries. */
export function toCodeCompletions(
  variables: readonly VariableSuggestion[],
  labels: VariableCompletionLabels,
): CodeCompletionItem[] {
  return variables.map((item) => {
    const scopeLabel = labelForScope(item.scope, labels);
    const preview = item.secret ? labels.redacted : item.value;
    const infoParts = [item.description.trim(), preview].filter((part) => part !== '');

    return {
      apply: `{{${item.key}}}`,
      detail: scopeLabel,
      info: infoParts.length > 0 ? infoParts.join(' · ') : undefined,
      label: item.key,
    };
  });
}

/** Visual token classification for URL templates and parameter syntax. */
export type UrlTokenKind = 'path' | 'query_key' | 'query_punct' | 'query_value' | 'text' | 'variable';

/** One classified text segment produced by `tokenizeUrlTemplate`. */
export interface UrlToken {
  readonly kind: UrlTokenKind;
  readonly text: string;
}

/** Live context from the Import OpenAPI dialog used to dismiss fixed warnings. */
export interface ImportWarningContext {
  readonly baseUrl: string;
  readonly cookies: readonly CookieView[];
}

const DOUBLE_BRACE_VAR = /\{\{[^{}\s]+\}\}/gu;
const URL_BASE_TOKEN = /\{\{[^{}\s]+\}\}|(?<!\{)\{[^{}/\s?&#]+\}(?!\})/gu;

function pushTokensWithVariables(text: string, fallbackKind: UrlTokenKind, out: UrlToken[]): void {
  let cursor = 0;

  for (const match of text.matchAll(DOUBLE_BRACE_VAR)) {
    const start = match.index;
    const end = start + match[0].length;

    if (start > cursor) {
      out.push({ kind: fallbackKind, text: text.slice(cursor, start) });
    }
    out.push({ kind: 'variable', text: match[0] });
    cursor = end;
  }

  if (cursor < text.length) {
    out.push({ kind: fallbackKind, text: text.slice(cursor) });
  }
}

/**
 * Splits a URL template into typed visual tokens so `{{variable}}`, `{path}`
 * and `?query=value` segments are rendered distinctly across views.
 */
export function tokenizeUrlTemplate(urlTemplate: string): UrlToken[] {
  if (urlTemplate === '') {
    return [];
  }

  const tokens: UrlToken[] = [];
  const { base, fragment, query } = splitUrlParts(urlTemplate);

  let cursor = 0;
  for (const match of base.matchAll(URL_BASE_TOKEN)) {
    const start = match.index;
    const end = start + match[0].length;

    if (start > cursor) {
      tokens.push({ kind: 'text', text: base.slice(cursor, start) });
    }
    const kind: UrlTokenKind = match[0].startsWith('{{') ? 'variable' : 'path';
    tokens.push({ kind, text: match[0] });
    cursor = end;
  }
  if (cursor < base.length) {
    tokens.push({ kind: 'text', text: base.slice(cursor) });
  }

  if (query !== null) {
    tokens.push({ kind: 'query_punct', text: '?' });
    const parts = query.split('&');

    for (let index = 0; index < parts.length; index += 1) {
      if (index > 0) {
        tokens.push({ kind: 'query_punct', text: '&' });
      }
      const part = parts[index] ?? '';
      if (part === '') {
        continue;
      }

      const eqIndex = part.indexOf('=');
      if (eqIndex === -1) {
        pushTokensWithVariables(part, 'query_key', tokens);
      } else {
        if (eqIndex > 0) {
          pushTokensWithVariables(part.slice(0, eqIndex), 'query_key', tokens);
        }
        tokens.push({ kind: 'query_punct', text: '=' });
        if (eqIndex + 1 < part.length) {
          pushTokensWithVariables(part.slice(eqIndex + 1), 'query_value', tokens);
        }
      }
    }
  }

  if (fragment !== null) {
    tokens.push({ kind: 'text', text: '#' });
    pushTokensWithVariables(fragment, 'text', tokens);
  }

  return tokens;
}

/** Extracts unique `{{variable}}` names referenced inside `text`. */
export function extractVariableNames(text: string): string[] {
  const seen = new Set<string>();
  const names: string[] = [];

  for (const match of text.matchAll(DOUBLE_BRACE_VAR)) {
    const inner = match[0].slice(2, -2).trim();
    const isNew = inner !== '' && !seen.has(inner);
    if (isNew) {
      seen.add(inner);
      names.push(inner);
    }
  }

  return names;
}

/**
 * Returns whether `raw` is a usable collection base URL (an `http://` or
 * `https://` URL with a host, or a `{{variable}}` template without spaces).
 */
export function hasUsableBaseUrl(raw: string): boolean {
  const trimmed = raw.trim();
  if (trimmed === '') {
    return false;
  }

  const hasWhitespace = /\s/u.test(trimmed);
  if (hasWhitespace) {
    return false;
  }

  if (/\{\{[^{}\s]+\}\}/u.test(trimmed)) {
    return true;
  }

  try {
    const parsed = new URL(trimmed);
    const isHttp = parsed.protocol === 'http:' || parsed.protocol === 'https:';
    const hasHost = parsed.hostname !== '';
    return isHttp && hasHost;
  } catch {
    return false;
  }
}

/**
 * Filters OpenAPI preview warnings to those still unresolved given the owner's
 * current base URL and Cookie Jar contents.
 */
export function activeImportWarnings(
  warnings: readonly OpenApiWarning[],
  context: ImportWarningContext,
): OpenApiWarning[] {
  const baseUrlReady = hasUsableBaseUrl(context.baseUrl);

  return warnings.filter((warning) => {
    switch (warning.kind) {
      case 'missing_base_url': {
        return !baseUrlReady;
      }
      case 'cookie_parameter':
      case 'cookie_security_scheme': {
        const expected = warning.cookieName.trim();
        const hasCookie = context.cookies.some((cookie) => {
          const isEnabled = cookie.enabled;
          const matchesName = cookie.name.trim() === expected;

          return isEnabled && matchesName;
        });
        return !hasCookie;
      }
      case 'combined_security_schemes':
      case 'cyclic_ref':
      case 'deep_ref_chain':
      case 'endpoint_limit_reached':
      case 'skipped_external_ref':
      case 'unresolved_local_ref':
      case 'unsupported_operation_security': {
        return true;
      }
    }
  });
}

/** Infers the syntax highlighting language from an HTTP Content-Type and optional payload sample. */
export function inferCodeLanguage(contentType: string | null | undefined, body = ''): CodeEditorLanguage {
  const normalized = (contentType ?? '').toLowerCase();

  if (normalized.includes('json')) {
    return 'json';
  }
  if (normalized.includes('yaml') || normalized.includes('yml')) {
    return 'yaml';
  }
  if (normalized.includes('html')) {
    return 'html';
  }
  if (normalized.includes('xml') || normalized.includes('svg')) {
    return 'xml';
  }

  const trimmed = body.trimStart();
  const looksJson = trimmed.startsWith('{') || trimmed.startsWith('[');

  if (looksJson) {
    try {
      JSON.parse(trimmed);
      return 'json';
    } catch {
      return 'plain';
    }
  }
  const lower = trimmed.toLowerCase();
  const looksHtml = lower.startsWith('<!doctype html') || lower.startsWith('<html');

  if (looksHtml) {
    return 'html';
  }
  if (trimmed.startsWith('<?xml')) {
    return 'xml';
  }
  return 'plain';
}
