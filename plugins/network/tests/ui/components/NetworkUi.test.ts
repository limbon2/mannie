// Server-side render tests for the Network Client UI components: empty/error
// states, collection overview, Postman-style request workbench, unredacted
// owner response pane and sanitized agent tool-call details card.
import { render } from 'svelte/server';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import AuthEditor from '../../../ui/components/AuthEditor.svelte';
import KeyValueEditor from '../../../ui/components/KeyValueEditor.svelte';
import NetworkToolDetails from '../../../ui/components/NetworkToolDetails.svelte';
import NetworkView from '../../../ui/components/NetworkView.svelte';
import ResponsePane from '../../../ui/components/ResponsePane.svelte';
import TokenizedUrl from '../../../ui/components/TokenizedUrl.svelte';
import VariableInput from '../../../ui/components/VariableInput.svelte';
import type { CollectionView, ManualResponseView, RequestView } from '../../../ui/generated/bindings.js';
import { networkLibrary } from '../../../ui/state/network.svelte.js';

const parameters = vi.hoisted(() => ({ search: new URLSearchParams() }));

vi.mock('@mannie/plugin-sdk/navigation', () => ({
  openView: vi.fn(),
  viewParameters: () => parameters.search,
}));

function sampleRequest(): RequestView {
  return {
    auth: { kind: 'inherit' },
    body: { content: '{"amount":100}', kind: 'json' },
    collectionId: 'col-1',
    createdAt: 1,
    description: 'Create a charge',
    folderId: null,
    headers: [
      {
        description: '',
        enabled: true,
        key: 'X-Trace',
        secret: false,
        value: 'abc',
      },
    ],
    id: 'req-1',
    method: 'POST',
    name: 'Create charge',
    pathParams: [
      {
        description: '',
        enabled: true,
        key: 'id',
        secret: false,
        value: 'ch_1',
      },
    ],
    protocol: 'http',
    queryParams: [],
    sortOrder: 0,
    timeoutSeconds: 30,
    updatedAt: 2,
    urlTemplate: '/charges/{id}',
    useCookies: true,
  };
}

function sampleCollection(): CollectionView {
  return {
    activeEnvironmentId: null,
    auth: { kind: 'bearer', token: 'sk_live_owner_visible_token' },
    autoCookies: true,
    baseUrl: 'https://api.example.com',
    createdAt: 1,
    description: 'Billing API',
    enabled: true,
    environments: [],
    folders: [],
    headers: [],
    id: 'col-1',
    name: 'Payments API',
    protocol: 'http',
    queryParams: [],
    requests: [sampleRequest()],
    updatedAt: 2,
    variables: [],
  };
}

beforeEach(() => {
  parameters.search = new URLSearchParams();
  networkLibrary.collections = [];
  networkLibrary.cookies = [];
  networkLibrary.globalVariables = [];
  networkLibrary.responses = {};
  networkLibrary.sendFailures = {};
  networkLibrary.runningRuns = {};
  networkLibrary.status = 'ready';
  networkLibrary.failure = null;
});

describe('NetworkView and workbench components', () => {
  it('renders empty state with New collection and Import OpenAPI actions', () => {
    const body = render(NetworkView).body;

    expect(body).toContain('data-network-empty');
    expect(body).toContain('data-network-new-collection');
    expect(body).toContain('data-network-import-openapi');
  });

  it('renders collection overview and request workbench when selected', () => {
    networkLibrary.collections = [sampleCollection()];
    parameters.search = new URLSearchParams('collection=col-1');

    const overview = render(NetworkView).body;
    expect(overview).toContain('data-network-collection="col-1"');
    expect(overview).toContain('Payments API');
    expect(overview).toContain('Create charge');

    parameters.search = new URLSearchParams('collection=col-1&request=req-1');
    const workbench = render(NetworkView).body;
    expect(workbench).toContain('data-network-workbench="req-1"');
    expect(workbench).toContain('data-network-send');
    expect(workbench).toContain('data-network-splitter');
    expect(workbench).toContain('/charges/{id}');
    expect(workbench).toContain('data-network-token-legend');
    expect(workbench).toContain('Query');
    expect(workbench).toContain('Path');
  });

  it('visually differentiates variables, path params and query params across components', () => {
    const urlHtml = render(TokenizedUrl, {
      props: { value: '{{base}}/items/{itemId}?page={{p}}&sort=asc' },
    }).body;
    expect(urlHtml).toContain('data-network-token="variable"');
    expect(urlHtml).toContain('data-network-token="path"');
    expect(urlHtml).toContain('data-network-token="query_key"');
    expect(urlHtml).toContain('data-network-token="query_value"');

    const pathTable = render(KeyValueEditor, {
      props: {
        allowAddRemove: false,
        entries: [{ description: '', enabled: true, key: 'itemId', secret: false, value: '42' }],
        kind: 'path',
        lockKeys: true,
        onChange: () => undefined,
        showEnabled: false,
      },
    }).body;
    expect(pathTable).toContain('data-network-token-kind="path"');
    expect(pathTable).toContain('{itemId}');

    const queryTable = render(KeyValueEditor, {
      props: {
        entries: [{ description: '', enabled: true, key: 'limit', secret: false, value: '20' }],
        kind: 'query',
        onChange: () => undefined,
      },
    }).body;
    expect(queryTable).toContain('data-network-token-kind="query"');

    const variableTable = render(KeyValueEditor, {
      props: {
        entries: [{ description: '', enabled: true, key: 'apiHost', secret: false, value: 'https://api.example.com' }],
        kind: 'variable',
        onChange: () => undefined,
      },
    }).body;
    expect(variableTable).toContain('data-network-token-kind="variable"');

    const varInput = render(VariableInput, {
      props: {
        onValueChange: () => undefined,
        value: 'Bearer {{token}}',
      },
    }).body;
    expect(varInput).toContain('data-network-variable-indicator');
  });

  it('omits redundant lock icons in AuthEditor and KeyValueEditor', () => {
    const authBody = render(AuthEditor, {
      props: {
        allowInherit: true,
        auth: { kind: 'bearer', token: '' },
        onChange: () => undefined,
      },
    }).body;
    expect(authBody).not.toContain('credential store');

    const kvBody = render(KeyValueEditor, {
      props: {
        allowSecret: true,
        entries: [
          {
            description: '',
            enabled: true,
            key: 'X-Key',
            secret: true,
            value: 'v',
          },
        ],
        onChange: () => undefined,
      },
    }).body;
    expect(kvBody).not.toContain('lucide-lock');
  });

  it('renders live unredacted owner response in ResponsePane', () => {
    const response: ManualResponseView = {
      body: '{"secret_echo":"sk_live_owner_visible_token"}',
      bytes: 45,
      contentType: 'application/json',
      cookies: [
        {
          domain: 'api.example.com',
          expiresAt: null,
          hostOnly: true,
          httpOnly: true,
          name: 'sid',
          path: '/',
          secure: true,
          value: 'owner_cookie_123',
        },
      ],
      durationMs: 38,
      headers: [{ name: 'content-type', value: 'application/json' }],
      isBinary: false,
      method: 'GET',
      protocol: 'http',
      status: 200,
      statusText: 'OK',
      truncated: false,
      url: 'https://api.example.com/v1/charges/ch_1',
    };

    const body = render(ResponsePane, {
      props: {
        failure: null,
        onCancel: () => undefined,
        response,
        running: false,
      },
    }).body;

    expect(body).toContain('200 OK');
    expect(body).toContain('sk_live_owner_visible_token');
  });

  it('renders sanitized agent tool card details with redacted URL', () => {
    const body = render(NetworkToolDetails, {
      props: {
        call: {
          arguments: {},
          data: {
            bytes: 128,
            collectionName: 'Payments API',
            durationMs: 120,
            method: 'POST',
            protocol: 'http',
            requestName: 'Create charge',
            status: 201,
            statusText: 'Created',
            truncated: false,
            url: 'https://api.example.com/charges/ch_1?token=[redacted]',
          },
          failure: null,
          status: 'completed',
        },
        conversationId: '0199e0f2-0000-7000-8000-000000000001',
      },
    }).body;

    expect(body).toContain('HTTP');
    expect(body).toContain('POST');
    expect(body).toContain('201 Created');
    expect(body).toContain('[redacted]');
  });
});
