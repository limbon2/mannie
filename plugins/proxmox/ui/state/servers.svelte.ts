// Registered Proxmox VE servers and their explicitly allowed QEMU VMs
// (plugins/proxmox/docs/README.md; ADR-0080). Follows the plugin's live
// channel only while the plugin is turned on (ADR-0034, decision 8).
import type { PluginContext } from '@mannie/plugin-sdk';
import { batchChannel, call, type ChannelMessage, type SubscriptionId, unsubscribe } from '@mannie/plugin-sdk/ipc';

import { channels, type ServerListUpdate, type ServerView } from '../generated/bindings.js';
import type { ProxmoxFailure } from '../types/proxmox.js';

/** Live list of registered Proxmox VE servers in this window. */
export class ProxmoxServerList {
  /** Why the list could not be read, if it could not. */
  failure: ProxmoxFailure | null = $state(null);
  /** Registered servers in display order. */
  servers: ServerView[] = $state([]);
  status: 'error' | 'loading' | 'ready' = $state('loading');
  #following = false;
  #generation = 0;
  #received = 0;
  #subscription: SubscriptionId | null = null;

  /** Looks up a server by ID, if present. */
  find(id: string): ServerView | undefined {
    return this.servers.find((server) => server.id === id);
  }

  /** Subscribes while the plugin is on; subsequent calls are no-ops. */
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

  /** Removes a deleted server optimistically before the channel snapshot arrives. */
  forget(id: string): void {
    this.servers = this.servers.filter((server) => server.id !== id);
  }

  /** Number of channel snapshots applied so far, used to avoid stale optimistic updates. */
  get received(): number {
    return this.#received;
  }

  /** Retries subscribing after an error. */
  retry(): void {
    void this.#subscribe();
  }

  /** Applies a command result optimistically if no newer channel update arrived since `received`. */
  upsert(server: ServerView, received: number): void {
    if (received !== this.#received) {
      return;
    }
    const index = this.servers.findIndex((known) => known.id === server.id);

    this.servers = index === -1 ? [...this.servers, server] : this.servers.with(index, server);
  }

  #apply(message: ChannelMessage<ServerListUpdate>): void {
    switch (message.kind) {
      case 'ended': {
        if (message.reason === 'lagged') {
          void this.#subscribe();
        }
        return;
      }
      case 'item': {
        this.#received += 1;
        this.servers = message.item.servers;
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
    this.servers = [];
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
    if (this.servers.length === 0) {
      this.status = 'loading';
    }
    const channel = batchChannel<ChannelMessage<ServerListUpdate>>(
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
    const outcome = await call(() => channels.followServers(channel));
    if (generation !== this.#generation) {
      if (outcome.ok) {
        void unsubscribe(outcome.data);
      }
      return;
    }
    if (outcome.ok) {
      this.#subscription = outcome.data;
      return;
    }
    this.failure = outcome.failure;
    this.status = 'error';
  }
}

/** Shared live Proxmox server list for this window. */
export const serverList = new ProxmoxServerList();
