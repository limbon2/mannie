// The plugins as the core reports them (docs/features/plugins.md): the list
// from the core first, then the whole list again after every change of a
// plugin's state, of one of its optional features or of the owner's
// choice. The shell's slots show a plugin's entries only while the plugin
// is on, and a feature's only while the plugin and the feature are on, so
// the window follows the list for as long as it runs (ADR-0030, decision
// 11; ADR-0055, decision 10).
import {
  commands,
  type FeatureView,
  type PluginListUpdate,
  type PluginsError,
  type PluginView,
  type PluginWork,
  type SubscriptionId,
} from '#lib/generated/bindings.js';
import type { Failure, Outcome } from '#lib/types/calls.js';
import type { SlotOwner } from '#lib/types/shell-slots.js';
import { batchChannel, call } from '#lib/utils/calls.js';

/** Something that follows whether one plugin, or one of its features, is on (`whileOn`, `whileFeatureOn`). */
interface Follower {
  readonly off: () => void;
  readonly on: () => void;
  readonly owner: SlotOwner;
  /** Whether the follower was last told that its plugin or feature is on. */
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
  /** Counts the lists that the channel brought, so that a command's result never covers a newer one. */
  #received = 0;
  /** Counts subscriptions, so that the updates of an ended one change nothing. */
  #generation = 0;
  #started = false;
  #subscription: SubscriptionId | null = null;

  /**
   * Whether a plugin, or a plugin's feature, is on or on its way there, so
   * that its entries come back. A feature that the owner keeps on comes
   * back while its plugin starts or is on, until its own start fails.
   */
  comesBack(owner: SlotOwner): boolean {
    const plugin = this.view(owner.plugin)?.state;
    const pluginComesBack = plugin === 'starting' || plugin === 'on';
    if (owner.feature === null) {
      return pluginComesBack;
    }
    const feature = this.featureView(owner.plugin, owner.feature);
    const isMoving = feature?.state === 'starting' || feature?.state === 'on';
    const waitsForPlugin = feature?.enabled === true && feature.state === 'off';
    return pluginComesBack && (isMoving || waitsForPlugin);
  }

  /** The optional feature of the plugin with these IDs, if the list has it. */
  featureView(plugin: string, feature: string): FeatureView | undefined {
    return this.view(plugin)?.features.find((view) => view.id === feature);
  }

  /** The work that a plugin's optional feature has in progress, which turning the feature off lets go. */
  featureWork(plugin: string, feature: string): Promise<Outcome<PluginWork, PluginsError>> {
    return call(() => commands.pluginFeatureWork({ featureId: feature, pluginId: plugin }));
  }

  /**
   * Whether the core reports the plugin and its optional feature as on. A
   * feature that the list does not have yet is not.
   */
  isFeatureOn(plugin: string, feature: string): boolean {
    const pluginIsOn = this.isOn(plugin);
    const featureIsOn = this.featureView(plugin, feature)?.state === 'on';
    return pluginIsOn && featureIsOn;
  }

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
    const received = this.#received;
    const outcome = await call(() => commands.setPluginEnabled({ enabled, pluginId: id }));
    if (!outcome.ok) {
      return outcome.failure;
    }
    this.#replace(outcome.data, received);
    return null;
  }

  /** Turns a plugin's optional feature on or off; returns why it could not, or `null`. */
  async setFeatureEnabled(plugin: string, feature: string, enabled: boolean): Promise<Failure<PluginsError> | null> {
    const received = this.#received;
    const outcome = await call(() =>
      commands.setPluginFeatureEnabled({ enabled, featureId: feature, pluginId: plugin }),
    );
    if (!outcome.ok) {
      return outcome.failure;
    }
    this.#replace(outcome.data, received);
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
   * Calls `on` now if the core reports the plugin and its optional feature
   * as on and each time they turn on later, and `off` each time they stop
   * being on, until the returned function stops following. A feature's UI
   * state follows the feature through it, so that its subscriptions run only
   * while the feature is on (ADR-0055, decision 10).
   */
  whileFeatureOn(plugin: string, feature: string, on: () => void, off: () => void): () => void {
    return this.#follow({ off, on, owner: { feature, plugin }, wasOn: false });
  }

  /**
   * Calls `on` now if the core reports the plugin as on and each time it
   * turns on later, and `off` each time it stops being on, until the
   * returned function stops following. A plugin's UI state follows its
   * plugin through it, so that its subscriptions run only while the plugin
   * is on (ADR-0030, decision 11).
   */
  whileOn(plugin: string, on: () => void, off: () => void): () => void {
    return this.#follow({ off, on, owner: { feature: null, plugin }, wasOn: false });
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
        this.#received += 1;
        this.plugins = update.plugins;
        this.status = 'ready';
        this.failure = null;
        this.#tellAll();
      }
    }
  }

  /**
   * Puts in a plugin that a command returned, before its update arrives
   * through the channel. When the channel brought a list after the command
   * started, that list may be newer than the result, and the channel brings
   * the list again after every change, so the result stays out.
   */
  #replace(view: PluginView, received: number): void {
    if (received !== this.#received) {
      return;
    }
    const index = this.plugins.findIndex((plugin) => plugin.id === view.id);
    if (index !== -1) {
      this.plugins = this.plugins.with(index, view);
      this.#tellAll();
    }
  }

  /** Adds a follower, tells it whether its plugin or feature is on, and returns what stops following. */
  #follow(follower: Follower): () => void {
    this.#followers.push(follower);
    this.#tell(follower);
    return () => {
      this.#followers = this.#followers.filter((other) => other !== follower);
    };
  }

  /** Tells a follower whether its plugin or feature is on, when that changed since it was last told. */
  #tell(follower: Follower): void {
    const { feature, plugin } = follower.owner;
    const isOn = feature === null ? this.isOn(plugin) : this.isFeatureOn(plugin, feature);
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
