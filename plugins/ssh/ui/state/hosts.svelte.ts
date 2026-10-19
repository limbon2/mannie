// The registered SSH hosts (plugins/ssh/docs/README.md, settings): the
// list from the core first, then the whole list again after every change.
// The window follows it while the plugin is on, since the host ends the
// plugin's channel when the plugin stops and refuses a new one while it is
// off (ADR-0034, decision 8).
import type { PluginContext } from '@mannie/plugin-sdk';
import { batchChannel, call, type ChannelMessage, type SubscriptionId, unsubscribe } from '@mannie/plugin-sdk/ipc';

import { channels, type HostListUpdate, type HostView } from '../generated/bindings.js';
import type { SshFailure } from '../types/ssh.js';

/** The window's list of SSH hosts. */
export class HostList {
  /** Why the list could not be read, if it could not. */
  failure: SshFailure | null = $state(null);
  /** The hosts, by name. */
  hosts: HostView[] = $state([]);
  status: 'error' | 'loading' | 'ready' = $state('loading');
  #following = false;
  /** Counts subscriptions, so that the updates of an ended one change nothing. */
  #generation = 0;
  #subscription: SubscriptionId | null = null;

  /** The host with this ID, if the list has it. */
  find(id: string): HostView | undefined {
    return this.hosts.find((host) => host.id === id);
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

  /** Forgets a host that a command removed, before the list arrives through the channel. */
  forget(id: string): void {
    this.hosts = this.hosts.filter((host) => host.id !== id);
  }

  /** Follows the list again after a failure. */
  retry(): void {
    void this.#subscribe();
  }

  /** Adds or replaces a host that a command returned, before the list arrives through the channel. */
  upsert(host: HostView): void {
    const index = this.hosts.findIndex((known) => known.id === host.id);
    this.hosts = index === -1 ? [...this.hosts, host] : this.hosts.with(index, host);
  }

  #apply(message: ChannelMessage<HostListUpdate>): void {
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
        this.hosts = message.item.hosts;
        this.status = 'ready';
        this.failure = null;
      }
    }
  }

  /** Ends the subscription and forgets the hosts, as the plugin stops. */
  #end(): void {
    this.#generation += 1;
    const previous = this.#subscription;
    this.#subscription = null;
    if (previous !== null) {
      void unsubscribe(previous);
    }
    this.hosts = [];
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
    if (this.hosts.length === 0) {
      this.status = 'loading';
    }
    const channel = batchChannel<ChannelMessage<HostListUpdate>>(
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
    const outcome = await call(() => channels.followHosts(channel));
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

/** The window's list of SSH hosts. */
export const hostList = new HostList();
