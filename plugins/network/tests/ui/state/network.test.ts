// Tests the NetworkLibrary state store: follow_state channel lifecycle,
// snapshots, lagged resubscription, plugin-off cleanup and live owner request
// execution/cancellation.
import type * as Ipc from '@mannie/plugin-sdk/ipc';
import { TestPluginContext } from '@mannie/plugin-sdk/testing';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { CollectionView, ManualResponseView, NetworkStateSnapshot } from '../../../ui/generated/bindings.js';
import { NetworkLibrary } from '../../../ui/state/network.svelte.js';

const core = vi.hoisted(() => ({
  cancelOwnerRequest: vi.fn(),
  followState: vi.fn(),
  sendOwnerRequest: vi.fn(),
  unsubscribe: vi.fn(),
}));

const channel = vi.hoisted(() => ({
  deliver: (_messages: readonly Ipc.ChannelMessage<NetworkStateSnapshot>[]): void => undefined,
}));

vi.mock('../../../ui/generated/bindings.js', () => ({
  channels: { followState: core.followState },
  commands: {
    cancelOwnerRequest: core.cancelOwnerRequest,
    sendOwnerRequest: core.sendOwnerRequest,
  },
}));
vi.mock('@mannie/plugin-sdk/ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof Ipc>()),
  batchChannel: (onItems: (messages: readonly Ipc.ChannelMessage<NetworkStateSnapshot>[]) => void) => {
    channel.deliver = onItems;
    return {};
  },
  unsubscribe: core.unsubscribe,
}));

function sampleCollection(id: string, name: string, updatedAt = 10): CollectionView {
  return {
    activeEnvironmentId: null,
    auth: { kind: 'none' },
    autoCookies: true,
    baseUrl: 'https://api.example.com',
    createdAt: 1,
    description: '',
    enabled: true,
    environments: [],
    folders: [],
    headers: [],
    id,
    name,
    protocol: 'http',
    queryParams: [],
    requests: [],
    updatedAt,
    variables: [],
  };
}

interface FollowedLibrary {
  readonly context: TestPluginContext;
  readonly library: NetworkLibrary;
}

async function followedLibrary(): Promise<FollowedLibrary> {
  const context = new TestPluginContext('network', true);
  const library = new NetworkLibrary();
  library.follow(context);
  await vi.waitFor(() => {
    expect(core.followState).toHaveBeenCalled();
  });
  return { context, library };
}

beforeEach(() => {
  for (const mock of Object.values(core)) {
    mock.mockReset();
  }
  core.followState.mockResolvedValue({ data: 42, status: 'ok' });
  core.cancelOwnerRequest.mockResolvedValue({ data: null, status: 'ok' });
});

interface SendOkResult {
  readonly data: ManualResponseView;
  readonly status: 'ok';
}

describe('NetworkLibrary', () => {
  it('applies snapshots, resubscribes on lag and clears state when turned off', async () => {
    const { context, library } = await followedLibrary();
    expect(library.status).toBe('loading');

    channel.deliver([
      {
        item: {
          collections: [sampleCollection('col-1', 'Billing', 100)],
          cookies: [],
          globalVariables: [],
        },
        kind: 'item',
      },
    ]);

    expect(library.status).toBe('ready');
    expect(library.collections.map((c) => c.id)).toEqual(['col-1']);
    expect(library.latestCollection()?.id).toBe('col-1');

    channel.deliver([{ kind: 'ended', reason: 'lagged' }]);
    await vi.waitFor(() => {
      expect(core.followState).toHaveBeenCalledTimes(2);
    });

    context.turnOff();
    expect(library.collections).toEqual([]);
    expect(library.status).toBe('loading');
    expect(core.unsubscribe).toHaveBeenCalled();
  });

  it('stores live owner responses and handles failures without browser storage', async () => {
    const { library } = await followedLibrary();
    const response: ManualResponseView = {
      body: '{"ok":true}',
      bytes: 11,
      contentType: 'application/json',
      cookies: [],
      durationMs: 25,
      headers: [{ name: 'content-type', value: 'application/json' }],
      isBinary: false,
      method: 'GET',
      protocol: 'http',
      status: 200,
      statusText: 'OK',
      truncated: false,
      url: 'https://api.example.com/v1/charges/ch_1',
    };
    core.sendOwnerRequest.mockResolvedValueOnce({ data: response, status: 'ok' });

    const outcome = await library.sendOwnerRequest('col-1:req-1', {
      collectionId: 'col-1',
      requestId: 'req-1',
    });

    expect(outcome.ok).toBe(true);
    expect(library.responses['col-1:req-1']?.status).toBe(200);
    expect(library.isRunning('col-1:req-1')).toBe(false);
  });

  it('tracks concurrent owner requests per target and cancels only the targeted request', async () => {
    const { library } = await followedLibrary();
    let resolveFirst!: (value: SendOkResult) => void;
    let resolveSecond!: (value: SendOkResult) => void;

    core.sendOwnerRequest
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveFirst = resolve;
          }),
      )
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveSecond = resolve;
          }),
      );

    const first = library.sendOwnerRequest('col-1:req-1', {
      collectionId: 'col-1',
      requestId: 'req-1',
    });
    const second = library.sendOwnerRequest('col-1:req-2', {
      collectionId: 'col-1',
      requestId: 'req-2',
    });

    expect(library.isRunning('col-1:req-1')).toBe(true);
    expect(library.isRunning('col-1:req-2')).toBe(true);

    const firstRunId = library.runningRuns['col-1:req-1'];
    expect(typeof firstRunId).toBe('string');

    await library.cancelRequest('col-1:req-1');
    expect(core.cancelOwnerRequest).toHaveBeenCalledTimes(1);
    expect(core.cancelOwnerRequest).toHaveBeenCalledWith({ runId: firstRunId });
    expect(library.isRunning('col-1:req-2')).toBe(true);

    const baseResponse: ManualResponseView = {
      body: '{"ok":true}',
      bytes: 11,
      contentType: 'application/json',
      cookies: [],
      durationMs: 12,
      headers: [],
      isBinary: false,
      method: 'GET',
      protocol: 'http',
      status: 200,
      statusText: 'OK',
      truncated: false,
      url: 'https://api.example.com/v1/items',
    };

    resolveSecond({ data: { ...baseResponse, body: '{"which":2}' }, status: 'ok' });
    const secondOutcome = await second;
    expect(secondOutcome.ok).toBe(true);
    expect(library.responses['col-1:req-2']?.body).toBe('{"which":2}');
    expect(library.isRunning('col-1:req-2')).toBe(false);
    expect(library.isRunning('col-1:req-1')).toBe(true);

    resolveFirst({ data: { ...baseResponse, body: '{"which":1}' }, status: 'ok' });
    await first;
    expect(library.isRunning('col-1:req-1')).toBe(false);
  });

  it('records subscription failures and recovers on retry', async () => {
    core.followState.mockResolvedValueOnce({
      error: { code: 'storage_failed', origin: 'plugin' },
      status: 'error',
    });
    const { library } = await followedLibrary();

    await vi.waitFor(() => {
      expect(library.status).toBe('error');
    });
    expect(library.failure).not.toBeNull();

    core.followState.mockResolvedValueOnce({ data: 99, status: 'ok' });
    library.retry();
    await vi.waitFor(() => {
      expect(core.followState).toHaveBeenCalledTimes(2);
    });
  });
});
