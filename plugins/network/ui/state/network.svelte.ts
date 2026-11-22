// Reactive state store for the Network Client plugin (ADR-0075).
//
// Follows `follow_state` while the plugin is on (ADR-0030, decision 11). Never
// writes secrets, tokens or cookies to localStorage or sessionStorage.
import type { PluginContext } from '@mannie/plugin-sdk';
import { batchChannel, call, type ChannelMessage, type SubscriptionId, unsubscribe } from '@mannie/plugin-sdk/ipc';

import {
  channels,
  type CollectionInput,
  type CollectionView,
  commands,
  type CookieInput,
  type CookieView,
  type EnvironmentInput,
  type EnvironmentView,
  type FolderCreateInput,
  type FolderView,
  type ManualResponseView,
  type ManualSendInput,
  type NetworkStateSnapshot,
  type OpenApiConfirmInput,
  type OpenApiPreview,
  type RequestInput,
  type RequestView,
  type VariableEntry,
} from '../generated/bindings.js';
import type { CollectionRequestPair, NetworkFailure, NetworkOutcome } from '../types/network.js';

function createRunId(): string {
  if (typeof crypto !== 'undefined') {
    if (typeof crypto.randomUUID === 'function') {
      return crypto.randomUUID();
    }
  }
  return `run-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}

function withoutKey<T>(map: Partial<Record<string, T>>, excluded: string): Partial<Record<string, T>> {
  const next: Partial<Record<string, T>> = {};
  for (const [key, value] of Object.entries(map)) {
    if (key !== excluded) {
      next[key] = value;
    }
  }
  return next;
}

/** Reactive state of collections, environments, variables, cookies and manual runs. */
export class NetworkLibrary {
  /** All saved collections (with owner-visible variables, headers and auth). */
  collections: CollectionView[] = $state.raw([]);
  /** All cookies in the owner's Cookie Jar (with owner-visible values). */
  cookies: CookieView[] = $state.raw([]);
  /** Why `follow_state` could not connect, if `status === 'error'`. */
  failure: NetworkFailure | null = $state.raw(null);
  /** Global variables shared across all collections. */
  globalVariables: VariableEntry[] = $state.raw([]);
  /** Live response from the most recent manual workbench send per request key. */
  responses: Partial<Record<string, ManualResponseView>> = $state.raw({});
  /** In-flight manual workbench run IDs keyed by request key (`collectionId:requestId`). */
  runningRuns: Partial<Record<string, string>> = $state.raw({});
  /** Failure from the most recent manual send for each request key. */
  sendFailures: Partial<Record<string, NetworkFailure>> = $state.raw({});
  /** `ready` once `follow_state` delivered its first snapshot. */
  status: 'error' | 'loading' | 'ready' = $state('loading');

  #following = false;
  #generation = 0;
  #subscription: SubscriptionId | null = null;

  /** Finds one collection by ID. */
  findCollection(id: string): CollectionView | undefined {
    return this.collections.find((collection) => collection.id === id);
  }

  /** Finds one request by ID, optionally scoped to one collection. */
  findRequest(requestId: string, collectionId?: string): CollectionRequestPair | undefined {
    if (collectionId !== undefined) {
      const collection = this.findCollection(collectionId);
      const request = collection?.requests.find((item) => item.id === requestId);
      const hasCollection = collection !== undefined;
      const hasRequest = request !== undefined;

      return hasCollection && hasRequest ? { collection, request } : undefined;
    }

    for (const collection of this.collections) {
      const request = collection.requests.find((item) => item.id === requestId);
      if (request !== undefined) {
        return { collection, request };
      }
    }
    return undefined;
  }

  /** Returns the most recently updated collection, if any. */
  latestCollection(): CollectionView | undefined {
    let latest: CollectionView | undefined;
    for (const collection of this.collections) {
      if (latest === undefined) {
        latest = collection;
      } else if (collection.updatedAt > latest.updatedAt) {
        latest = collection;
      }
    }
    return latest;
  }

  /** Follows `follow_state` while the plugin is on. */
  follow(plugin: Pick<PluginContext, 'whileOn'>): void {
    if (this.#following) {
      return;
    }
    this.#following = true;
    plugin.whileOn(
      () => void this.#subscribe(),
      () => {
        this.#end();
      },
    );
  }

  /** Retries subscribing after a connection failure. */
  retry(): void {
    void this.#subscribe();
  }

  createCollection(input: CollectionInput): Promise<NetworkOutcome<CollectionView>> {
    return call(() => commands.createCollection(input));
  }

  updateCollection(collectionId: string, input: CollectionInput): Promise<NetworkOutcome<CollectionView>> {
    return call(() => commands.updateCollection({ collectionId, input }));
  }

  setCollectionEnabled(collectionId: string, enabled: boolean): Promise<NetworkOutcome<null>> {
    return call(() => commands.setCollectionEnabled({ collectionId, enabled }));
  }

  deleteCollection(collectionId: string): Promise<NetworkOutcome<null>> {
    return call(() => commands.deleteCollection({ collectionId }));
  }

  createFolder(input: FolderCreateInput): Promise<NetworkOutcome<FolderView>> {
    return call(() => commands.createFolder(input));
  }

  renameFolder(folderId: string, name: string): Promise<NetworkOutcome<null>> {
    return call(() => commands.renameFolder({ folderId, name }));
  }

  deleteFolder(folderId: string): Promise<NetworkOutcome<null>> {
    return call(() => commands.deleteFolder({ folderId }));
  }

  createRequest(collectionId: string, input: RequestInput): Promise<NetworkOutcome<RequestView>> {
    return call(() => commands.createRequest({ collectionId, input }));
  }

  updateRequest(requestId: string, input: RequestInput): Promise<NetworkOutcome<RequestView>> {
    return call(() => commands.updateRequest({ input, requestId }));
  }

  duplicateRequest(requestId: string): Promise<NetworkOutcome<RequestView>> {
    return call(() => commands.duplicateRequest({ requestId }));
  }

  deleteRequest(requestId: string): Promise<NetworkOutcome<null>> {
    return call(() => commands.deleteRequest({ requestId }));
  }

  saveEnvironment(input: EnvironmentInput): Promise<NetworkOutcome<EnvironmentView>> {
    return call(() => commands.saveEnvironment(input));
  }

  deleteEnvironment(environmentId: string): Promise<NetworkOutcome<null>> {
    return call(() => commands.deleteEnvironment({ environmentId }));
  }

  setActiveEnvironment(collectionId: string, environmentId: string | null): Promise<NetworkOutcome<null>> {
    return call(() => commands.setActiveEnvironment({ collectionId, environmentId }));
  }

  saveGlobalVariables(variables: VariableEntry[]): Promise<NetworkOutcome<VariableEntry[]>> {
    return call(() => commands.saveGlobalVariables({ variables }));
  }

  saveCookie(input: CookieInput): Promise<NetworkOutcome<CookieView>> {
    return call(() => commands.saveCookie(input));
  }

  deleteCookie(cookieId: string): Promise<NetworkOutcome<null>> {
    return call(() => commands.deleteCookie({ cookieId }));
  }

  clearCookies(): Promise<NetworkOutcome<null>> {
    return call(() => commands.clearCookies());
  }

  pickOpenApiFile(filterName: string): Promise<NetworkOutcome<OpenApiPreview | null>> {
    return call(() => commands.pickOpenapiFile({ filterName }));
  }

  previewOpenApiText(text: string): Promise<NetworkOutcome<OpenApiPreview>> {
    return call(() => commands.previewOpenapiText({ text }));
  }

  confirmOpenApiImport(input: OpenApiConfirmInput): Promise<NetworkOutcome<CollectionView>> {
    return call(() => commands.confirmOpenapiImport(input));
  }

  /** Whether a manual workbench send is currently running for `targetKey`. */
  isRunning(targetKey: string): boolean {
    return this.runningRuns[targetKey] !== undefined;
  }

  /** Runs an HTTP request directly from the owner workbench and records its live response. */
  async sendOwnerRequest(
    targetKey: string,
    input: Omit<ManualSendInput, 'runId'>,
  ): Promise<NetworkOutcome<ManualResponseView>> {
    const previousRunId = this.runningRuns[targetKey];
    if (previousRunId !== undefined) {
      void call(() => commands.cancelOwnerRequest({ runId: previousRunId }));
    }

    const runId = createRunId();

    this.runningRuns = { ...this.runningRuns, [targetKey]: runId };
    this.sendFailures = withoutKey(this.sendFailures, targetKey);

    const outcome = await call(() =>
      commands.sendOwnerRequest({
        ...input,
        runId,
      }),
    );

    if (this.runningRuns[targetKey] !== runId) {
      return outcome;
    }

    this.runningRuns = withoutKey(this.runningRuns, targetKey);

    if (outcome.ok) {
      this.responses = { ...this.responses, [targetKey]: outcome.data };
    } else {
      this.sendFailures = { ...this.sendFailures, [targetKey]: outcome.failure };
    }

    return outcome;
  }

  /** Cancels the currently running manual workbench request for `targetKey`, if any. */
  async cancelRequest(targetKey: string): Promise<void> {
    const runId = this.runningRuns[targetKey];
    if (runId === undefined) {
      return;
    }
    await call(() => commands.cancelOwnerRequest({ runId }));
  }

  /** Cancels every currently running manual workbench request. */
  async cancelAllRequests(): Promise<void> {
    const runIds = Object.values(this.runningRuns).filter((id): id is string => id !== undefined);
    await Promise.all(runIds.map((runId) => call(() => commands.cancelOwnerRequest({ runId }))));
  }

  #apply(message: ChannelMessage<NetworkStateSnapshot>): void {
    switch (message.kind) {
      case 'ended': {
        if (message.reason === 'lagged') {
          void this.#subscribe();
        }
        return;
      }
      case 'item': {
        this.collections = message.item.collections;
        this.globalVariables = message.item.globalVariables;
        this.cookies = message.item.cookies;
        this.status = 'ready';
        this.failure = null;
      }
    }
  }

  #end(): void {
    this.#generation += 1;
    const previous = this.#subscription;

    this.#subscription = null;
    if (previous !== null) {
      void unsubscribe(previous);
    }
    this.collections = [];
    this.globalVariables = [];
    this.cookies = [];
    this.responses = {};
    this.sendFailures = {};
    this.runningRuns = {};
    this.failure = null;
    this.status = 'loading';
  }

  async #subscribe(): Promise<void> {
    this.#generation += 1;
    const generation = this.#generation;
    const previous = this.#subscription;

    this.#subscription = null;
    if (previous !== null) {
      void unsubscribe(previous);
    }
    if (this.status === 'error') {
      this.status = 'loading';
    }
    const channel = batchChannel<ChannelMessage<NetworkStateSnapshot>>(
      (items) => {
        if (generation !== this.#generation) {
          return;
        }
        for (const item of items) {
          this.#apply(item);
        }
      },
      () => {
        if (generation === this.#generation) {
          void this.#subscribe();
        }
      },
    );
    const outcome = await call(() => channels.followState(channel));
    if (generation !== this.#generation) {
      if (outcome.ok) {
        void unsubscribe(outcome.data);
      }
      return;
    }
    if (outcome.ok) {
      this.#subscription = outcome.data;
      this.failure = null;
      return;
    }
    this.failure = outcome.failure;
    this.status = 'error';
  }
}

/** Shared singleton store for the Network Client plugin UI. */
export const networkLibrary = new NetworkLibrary();
