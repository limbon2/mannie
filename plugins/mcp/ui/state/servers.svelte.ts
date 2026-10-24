// The registered MCP servers (plugins/mcp/docs/README.md, settings): the
// list from the core first, then the whole list again after every change,
// with how many tools of all servers are on. The window follows it while the
// plugin is on, since the host ends the plugin's channel when the plugin
// stops and refuses a new one while it is off (ADR-0034, decision 8).
import type { PluginContext } from '@mannie/plugin-sdk';
import { batchChannel, call, type ChannelMessage, type SubscriptionId, unsubscribe } from '@mannie/plugin-sdk/ipc';

import { channels, type ServerListUpdate, type ServerView } from '../generated/bindings.js';
import type { McpFailure } from '../types/mcp.js';

/** The window's list of MCP servers. */
export class ServerList {
  /** How many tools of all servers are on. */
  enabledTools = $state(0);
  /** Why the list could not be read, if it could not. */
  failure: McpFailure | null = $state(null);
  /** The servers, by name. */
  servers: ServerView[] = $state([]);
  status: 'error' | 'loading' | 'ready' = $state('loading');
  #following = false;
  /** Counts subscriptions, so that the updates of an ended one change nothing. */
  #generation = 0;
  #subscription: SubscriptionId | null = null;

  /** The server with this ID, if the list has it. */
  find(id: string): ServerView | undefined {
    return this.servers.find((server) => server.id === id);
  }

  /** Follows the list while the plugin's context reports the plugin as on. Later calls change nothing. */
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

  /** Forgets a server that a command removed, before the list arrives through the channel. */
  forget(id: string): void {
    this.servers = this.servers.filter((server) => server.id !== id);
  }

  /** Follows the list again after a failure. */
  retry(): void {
    void this.#subscribe();
  }

  /** Adds or replaces a server that a command returned, before the list arrives through the channel. */
  upsert(server: ServerView): void {
    const index = this.servers.findIndex((known) => known.id === server.id);
    this.servers = index === -1 ? [...this.servers, server] : this.servers.with(index, server);
  }

  #apply(message: ChannelMessage<ServerListUpdate>): void {
    switch (message.kind) {
      case 'ended': {
        // A list that fell behind follows the list again. When the plugin
        // stops, the list ends as the plugin's state leaves "on".
        if (message.reason === 'lagged') {
          void this.#subscribe();
        }
        return;
      }
      case 'item': {
        this.servers = message.item.servers;
        this.enabledTools = message.item.enabledTools;
        this.status = 'ready';
        this.failure = null;
      }
    }
  }

  /** Ends the subscription and forgets the servers, as the plugin stops. */
  #end(): void {
    this.#generation += 1;
    const previous = this.#subscription;
    this.#subscription = null;
    if (previous !== null) {
      void unsubscribe(previous);
    }
    this.servers = [];
    this.enabledTools = 0;
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
      // A newer subscription replaced this one while it started.
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

/** The window's list of MCP servers. */
export const serverList = new ServerList();
