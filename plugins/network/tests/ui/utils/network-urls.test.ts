// Tests URL query/path synchronization, variable autocomplete collection and
// CodeMirror language inference for the Network Client UI.
import { describe, expect, it } from 'vitest';

import type { CookieView, KeyValueEntry, OpenApiWarning } from '../../../ui/generated/bindings.js';
import {
  activeImportWarnings,
  availableVariables,
  composeUrlWithQuery,
  extractPathParamNames,
  extractVariableNames,
  hasUsableBaseUrl,
  inferCodeLanguage,
  stripQueryFromUrl,
  syncPathParamsFromUrl,
  syncQueryParamsFromUrl,
  toCodeCompletions,
  tokenizeUrlTemplate,
} from '../../../ui/utils/network-urls.js';

interface EntryOverrides extends Partial<KeyValueEntry> {
  readonly key: string;
  readonly value: string;
}

function entry(overrides: EntryOverrides): KeyValueEntry {
  return {
    description: '',
    enabled: true,
    secret: false,
    ...overrides,
  };
}

describe('path parameter extraction and one-way sync', () => {
  it('extracts unique {param} placeholders while ignoring {{variables}}, query and fragments', () => {
    expect(extractPathParamNames('{{baseUrl}}/v1/orgs/{orgId}/users/{user_id}/items/{orgId}?filter={q}#{h}')).toEqual([
      'orgId',
      'user_id',
    ]);
    expect(extractPathParamNames('https://api.example.com/static/path')).toEqual([]);
  });

  it('syncs path params from the URL and restores remembered values when re-added', () => {
    const remembered = new Map<string, KeyValueEntry>();
    const initial: KeyValueEntry[] = [
      entry({ description: 'Org slug', key: 'orgId', value: 'acme' }),
      entry({ key: 'userId', secret: true, value: 'u_99' }),
    ];

    const removed = syncPathParamsFromUrl('/v1/orgs/{orgId}', initial, remembered);
    expect(removed).toEqual([entry({ description: 'Org slug', key: 'orgId', value: 'acme' })]);
    expect(remembered.get('userId')?.value).toBe('u_99');

    const readded = syncPathParamsFromUrl('/v1/orgs/{orgId}/users/{userId}/posts/{postId}', removed, remembered);
    expect(readded).toEqual([
      entry({ description: 'Org slug', key: 'orgId', value: 'acme' }),
      entry({ key: 'userId', secret: true, value: 'u_99' }),
      entry({ key: 'postId', value: '' }),
    ]);
  });
});

describe('query parameter bidirectional sync and URL stripping', () => {
  it('strips query parameters while preserving fragment and template braces', () => {
    expect(stripQueryFromUrl('{{baseUrl}}/items?a=1&b=2#top')).toBe('{{baseUrl}}/items#top');
    expect(stripQueryFromUrl('/items?only=query')).toBe('/items');
    expect(stripQueryFromUrl('/items')).toBe('/items');
  });

  it('composes enabled query parameters into the URL without encoding {{var}} templates', () => {
    const url = composeUrlWithQuery('/search#results', [
      entry({ key: 'q', value: '{{query}}' }),
      entry({ enabled: false, key: 'debug', value: '1' }),
      entry({ key: 'limit', value: '25' }),
    ]);
    expect(url).toBe('/search?q={{query}}&limit=25#results');

    expect(composeUrlWithQuery('/search?old=1', [entry({ enabled: false, key: 'x', value: '1' })])).toBe('/search');
  });

  it('syncs edited URL query params back into the table while preserving disabled rows and secret flags', () => {
    const existing: KeyValueEntry[] = [
      entry({ description: 'API token', key: 'token', secret: true, value: 'old' }),
      entry({ enabled: false, key: 'verbose', value: 'true' }),
    ];

    const synced = syncQueryParamsFromUrl('/search?token=new_val&page=2', existing);
    expect(synced).toEqual([
      entry({ description: 'API token', key: 'token', secret: true, value: 'new_val' }),
      entry({ key: 'page', value: '2' }),
      entry({ enabled: false, key: 'verbose', value: 'true' }),
    ]);
  });
});

describe('variable suggestions and code editor completions', () => {
  it('merges environment, collection and global variables in precedence order and hides secrets in completions', () => {
    const collection = {
      environments: [
        {
          collectionId: 'col-1',
          createdAt: 1,
          id: 'env-prod',
          name: 'Prod',
          updatedAt: 1,
          variables: [
            entry({ key: 'baseUrl', value: 'https://prod.example.com' }),
            entry({ key: 'apiKey', secret: true, value: 'sk_live_secret' }),
            entry({ enabled: false, key: 'disabledVar', value: 'nope' }),
          ],
        },
      ],
      variables: [
        entry({ key: 'baseUrl', value: 'https://staging.example.com' }),
        entry({ description: 'Default version', key: 'apiVersion', value: 'v2' }),
      ],
    };

    const globals: KeyValueEntry[] = [
      entry({ key: 'apiVersion', value: 'v1' }),
      entry({ key: 'region', value: 'eu-west' }),
    ];

    const vars = availableVariables(collection, 'env-prod', globals);
    expect(vars.map((v) => `${v.key}:${v.scope}:${v.value}`)).toEqual([
      'baseUrl:environment:https://prod.example.com',
      'apiKey:environment:sk_live_secret',
      'apiVersion:collection:v2',
      'region:global:eu-west',
    ]);

    const completions = toCodeCompletions(vars, {
      collection: 'Collection',
      environment: 'Environment',
      global: 'Global',
      redacted: '[redacted]',
    });
    const secretCompletion = completions.find((item) => item.label === 'apiKey');
    const publicCompletion = completions.find((item) => item.label === 'region');
    expect(secretCompletion?.info).toBe('[redacted]');
    expect(secretCompletion?.info).not.toContain('sk_live_secret');
    expect(publicCompletion?.info).toBe('eu-west');
  });
});

describe('inferCodeLanguage', () => {
  it('detects JSON, YAML, XML, HTML and plain text from content type or body', () => {
    expect(inferCodeLanguage('application/vnd.api+json; charset=utf-8', '{}')).toBe('json');
    expect(inferCodeLanguage('application/x-yaml', 'openapi: 3.1.0')).toBe('yaml');
    expect(inferCodeLanguage('text/html', '<html></html>')).toBe('html');
    expect(inferCodeLanguage('application/xml', '<root />')).toBe('xml');
    expect(inferCodeLanguage(null, '{\n  "ok": true\n}')).toBe('json');
    expect(inferCodeLanguage(null, '<!DOCTYPE html><body></body>')).toBe('html');
    expect(inferCodeLanguage(null, '<?xml version="1.0"?><root/>')).toBe('xml');
    expect(inferCodeLanguage('text/plain', 'plain output')).toBe('plain');
  });
});

describe('tokenizeUrlTemplate and extractVariableNames', () => {
  it('distinguishes variables, path parameters, query keys, punctuation and values', () => {
    expect(tokenizeUrlTemplate('{{base}}/orgs/{orgId}/items/{itemId}?page={{page}}&sort=asc#{{section}}')).toEqual([
      { kind: 'variable', text: '{{base}}' },
      { kind: 'text', text: '/orgs/' },
      { kind: 'path', text: '{orgId}' },
      { kind: 'text', text: '/items/' },
      { kind: 'path', text: '{itemId}' },
      { kind: 'query_punct', text: '?' },
      { kind: 'query_key', text: 'page' },
      { kind: 'query_punct', text: '=' },
      { kind: 'variable', text: '{{page}}' },
      { kind: 'query_punct', text: '&' },
      { kind: 'query_key', text: 'sort' },
      { kind: 'query_punct', text: '=' },
      { kind: 'query_value', text: 'asc' },
      { kind: 'text', text: '#' },
      { kind: 'variable', text: '{{section}}' },
    ]);

    expect(tokenizeUrlTemplate('')).toEqual([]);
  });

  it('extracts unique referenced {{variable}} names in order', () => {
    expect(extractVariableNames('Bearer {{token}} for {{tenant}} ({{token}})')).toEqual(['token', 'tenant']);
    expect(extractVariableNames('/users/{id}')).toEqual([]);
  });
});

describe('hasUsableBaseUrl and activeImportWarnings', () => {
  it('accepts valid HTTP(S) URLs and {{variable}} templates and rejects empty or malformed values', () => {
    expect(hasUsableBaseUrl('https://api.example.com/v1')).toBe(true);
    expect(hasUsableBaseUrl('http://127.0.0.1:8080')).toBe(true);
    expect(hasUsableBaseUrl('{{apiBaseUrl}}')).toBe(true);
    expect(hasUsableBaseUrl('{{origin}}/v2')).toBe(true);
    expect(hasUsableBaseUrl('')).toBe(false);
    expect(hasUsableBaseUrl('   ')).toBe(false);
    expect(hasUsableBaseUrl('not-a-url')).toBe(false);
    expect(hasUsableBaseUrl('ftp://files.example.com')).toBe(false);
    expect(hasUsableBaseUrl('https://api.example.com /bad')).toBe(false);
  });

  it('dismisses missing_base_url and cookie warnings once fixed while keeping non-fixable notes', () => {
    const warnings: readonly OpenApiWarning[] = [
      { kind: 'missing_base_url', source: 'v3_servers' },
      { cookieName: 'sid', endpoint: 'GET /me', kind: 'cookie_parameter' },
      { cookieName: 'auth_cookie', kind: 'cookie_security_scheme', scheme: 'cookieAuth' },
      { kind: 'cyclic_ref', reference: '#/components/schemas/Node' },
    ];
    const cookies: readonly CookieView[] = [
      {
        createdAt: 1,
        domain: 'api.example.com',
        enabled: true,
        expiresAt: null,
        hostOnly: true,
        httpOnly: true,
        id: 'c-1',
        name: 'sid',
        path: '/',
        secure: true,
        updatedAt: 1,
        value: 'secret',
      },
      {
        createdAt: 1,
        domain: 'api.example.com',
        enabled: false,
        expiresAt: null,
        hostOnly: true,
        httpOnly: true,
        id: 'c-2',
        name: 'auth_cookie',
        path: '/',
        secure: true,
        updatedAt: 1,
        value: 'disabled',
      },
    ];

    expect(activeImportWarnings(warnings, { baseUrl: '', cookies: [] }).map((item) => item.kind)).toEqual([
      'missing_base_url',
      'cookie_parameter',
      'cookie_security_scheme',
      'cyclic_ref',
    ]);

    expect(
      activeImportWarnings(warnings, { baseUrl: 'https://api.example.com', cookies }).map((item) => item.kind),
    ).toEqual(['cookie_security_scheme', 'cyclic_ref']);

    expect(
      activeImportWarnings([{ kind: 'missing_base_url', source: 'v3_servers' }], {
        baseUrl: 'https://api.example.com',
        cookies: [],
      }),
    ).toEqual([]);
  });
});
