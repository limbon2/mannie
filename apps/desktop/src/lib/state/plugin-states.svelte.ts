// The plugins as the core reports them (docs/features/plugins.md): the list
// from the core first, then the whole list again after every change of a
// plugin's state or of the owner's choice. The shell's slots show a
// plugin's entries only while the plugin is on, so the window follows the
// list for as long as it runs (ADR-0030, decision 11).
import {
  commands,
  type PluginListUpdate,
  type PluginsError,
  type PluginView,
  type PluginWork,
  type SubscriptionId,
} from '#lib/generated/bindings.js';
import type { Failure, Outcome } from '#lib/types/calls.js';
import { batchChannel, call } from '#lib/utils/calls.js';

/** Something that follows whether one plugin is on (`whileOn`). */
interface Follower {
  readonly off: () => void;
  readonly on: () => void;
  readonly plugin: string;
  /** Whether the follower was last told that the plugin is on. */
  wasOn: boolean;
}

/** The window's list of plugins. */
export class PluginStates {
  /** Why the list could not be read, if it could not. */
  failure: Failure<PluginsError> | null = $state(null);
  /** The plugins, in the order that Mannie lists them. */
  plugins: PluginView[] = $state([]);
  status: 'error' | 'loading' | 'ready' = $state('loading');
  #followers: Follower[] = [];
  /** Counts subscriptions, so that the updates of an ended one change nothing. */
  #generation = 0;
  #started = false;
  #subscription: SubscriptionId | null = null;

  /** Whether the core reports the plugin as on. A plugin that the list does not have yet is not. */
  isOn(id: string): boolean {
    return this.view(id)?.state === 'on';
  }

  /** Follows the list again after a failure. */
  retry(): void {
    void this.#subscribe();
  }

  /** Turns a plugin on or off; returns why it could not, or `null`. */
  async setEnabled(id: string, enabled: boolean): Promise<Failure<PluginsError> | null> {
    const outcome = await call(() => commands.setPluginEnabled({ enabled, pluginId: id }));
    if (!outcome.ok) {
      return outcome.failure;
    }
    this.#replace(outcome.data);
    return null;
  }

  /** Starts following the list. Later calls change nothing. */
  start(): void {
    if (this.#started) {
      return;
    }
    this.#started = true;
    void this.#subscribe();
  }

  /**
   * Calls `on` now if the core reports the plugin as on and each time it
   * turns on later, and `off` each time it stops being on, until the
   * returned function stops following. A plugin's UI state follows its
   * plugin through it, so that its subscriptions run only while the plugin
   * is on (ADR-0030, decision 11).
   */
  whileOn(plugin: string, on: () => void, off: () => void): () => void {
    const follower: Follower = { off, on, plugin, wasOn: false };
    this.#followers.push(follower);
    this.#tell(follower);
    return () => {
      this.#followers = this.#followers.filter((other) => other !== follower);
    };
  }

  /** The plugin with this ID, if the list has it. */
  view(id: string): PluginView | undefined {
    return this.plugins.find((plugin) => plugin.id === id);
  }

  /** The work that a plugin has in progress, which turning it off lets go. */
  work(id: string): Promise<Outcome<PluginWork, PluginsError>> {
    return call(() => commands.pluginWork({ pluginId: id }));
  }

  #apply(update: PluginListUpdate): void {
    switch (update.type) {
      case 'ended': {
        // A list that fell behind follows the list again; at shutdown the
        // window is about to close.
        if (update.reason === 'lagged') {
          void this.#subscribe();
        }
        return;
      }
      case 'plugins': {
        this.plugins = update.plugins;
        this.status = 'ready';
        this.failure = null;
        this.#tellAll();
      }
    }
  }

  /** Puts in a plugin that a command returned, before its update arrives through the channel. */
  #replace(view: PluginView): void {
    const index = this.plugins.findIndex((plugin) => plugin.id === view.id);
    if (index !== -1) {
      this.plugins = this.plugins.with(index, view);
      this.#tellAll();
    }
  }

  /** Tells a follower whether its plugin is on, when that changed since it was last told. */
  #tell(follower: Follower): void {
    const isOn = this.isOn(follower.plugin);
    if (isOn === follower.wasOn) {
      return;
    }
    follower.wasOn = isOn;
    if (isOn) {
      follower.on();
    } else {
      follower.off();
    }
  }

  #tellAll(): void {
    for (const follower of this.#followers) {
      this.#tell(follower);
    }
  }

  async #subscribe(): Promise<void> {
    this.#generation += 1;
    const generation = this.#generation;
    const previous = this.#subscription;
    this.#subscription = null;
    if (previous !== null) {
      void commands.unsubscribe({ subscriptionId: previous });
    }
    if (this.plugins.length === 0) {
      this.status = 'loading';
    }
    const channel = batchChannel<PluginListUpdate>(
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
    const outcome = await call(() => commands.subscribePlugins(channel));
    if (generation !== this.#generation) {
      // A newer subscription replaced this one while it started.
      if (outcome.ok) {
        void commands.unsubscribe({ subscriptionId: outcome.data });
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

/** The window's list of plugins. */
export const pluginStates = new PluginStates();
