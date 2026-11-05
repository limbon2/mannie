// Test helpers of the TypeScript plugin SDK, `@mannie/plugin-sdk/testing`
// (ADR-0034, decision 11): a plugin's tests register its UI in a context of
// their own, with the shell's real slots, and turn the plugin and its
// optional features on and off as the core would report them (ADR-0055,
// decision 11). Only tests import this module.
import { ShellSlots } from '#lib/state/shell-slots.svelte.js';

import type { FeatureContext, PluginContext } from './index.js';
import type { PluginSlots } from './slots.js';

/** The images of a message box, the target of an image source's files. */
export { MessageImages } from '#lib/state/message-images.svelte.js';

/** Something that follows whether a plugin or a feature is on. */
interface Follower {
  readonly off: () => void;
  readonly on: () => void;
  /** Whether the follower was last told that it is on. */
  wasOn: boolean;
}

/** Followers of one plugin or feature, each told when whether it is on changes. */
class Followers {
  readonly #isOn: () => boolean;
  #list: Follower[] = [];

  constructor(isOn: () => boolean) {
    this.#isOn = isOn;
  }

  /** Adds a follower, tells it now if it is on, and returns what stops following. */
  add(on: () => void, off: () => void): () => void {
    const follower: Follower = { off, on, wasOn: false };
    this.#list.push(follower);
    this.#tell(follower);
    return () => {
      this.#list = this.#list.filter((other) => other !== follower);
    };
  }

  /** Tells each follower whether it is on, when that changed since it was last told. */
  tellAll(): void {
    for (const follower of this.#list) {
      this.#tell(follower);
    }
  }

  #tell(follower: Follower): void {
    const isOn = this.#isOn();
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
}

/**
 * The context of a plugin's optional feature in a test, which is on only
 * while the test turns it on and its plugin is on.
 */
export class TestFeatureContext implements FeatureContext {
  readonly id: string;
  readonly slots: PluginSlots;
  readonly #followers: Followers;
  #on = false;
  readonly #pluginIsOn: () => boolean;

  /** A feature of a plugin whose context reports with `pluginIsOn` whether the plugin is on. */
  constructor(id: string, slots: PluginSlots, pluginIsOn: () => boolean) {
    this.id = id;
    this.slots = slots;
    this.#pluginIsOn = pluginIsOn;
    this.#followers = new Followers(() => this.isOn());
  }

  readonly isOn = (): boolean => this.#pluginIsOn() && this.#on;

  readonly whileOn = (on: () => void, off: () => void): (() => void) => this.#followers.add(on, off);

  /** Tells the followers whether the feature is on, after its plugin turned on or off. */
  followPlugin(): void {
    this.#followers.tellAll();
  }

  /** Reports the feature as off, as the core does when the owner turns it off. */
  turnOff(): void {
    this.#on = false;
    this.#followers.tellAll();
  }

  /** Reports the feature as on, as the core does once it started on its plugin. */
  turnOn(): void {
    this.#on = true;
    this.#followers.tellAll();
  }
}

/** A plugin's context in a test, whose plugin and features are on only while the test says so. */
export class TestPluginContext implements PluginContext {
  readonly id: string;
  /** The shell's slots, where the test finds the plugin's entries and its features'. */
  readonly shell: ShellSlots;
  readonly slots: PluginSlots;
  readonly #features = new Map<string, TestFeatureContext>();
  readonly #followers: Followers;
  #on: boolean;

  /** The context of the plugin `id`, which starts off unless `on` says otherwise. */
  constructor(id: string, on = false) {
    this.id = id;
    this.#on = on;
    this.#followers = new Followers(() => this.#on);
    this.shell = new ShellSlots(({ feature, plugin }) => {
      const isThisPlugin = plugin === id;
      if (!isThisPlugin) {
        return false;
      }
      return feature === null ? this.#on : (this.#features.get(feature)?.isOn() ?? false);
    });
    this.slots = this.shell.forPlugin(id);
  }

  /** The context of the plugin's feature `id`, the same one each time, which starts off. */
  readonly feature = (id: string): TestFeatureContext => {
    const known = this.#features.get(id);
    if (known !== undefined) {
      return known;
    }
    const context = new TestFeatureContext(id, this.shell.forFeature(this.id, id), () => this.#on);
    this.#features.set(id, context);
    return context;
  };

  readonly isOn = (): boolean => this.#on;

  readonly whileOn = (on: () => void, off: () => void): (() => void) => this.#followers.add(on, off);

  /** Reports the plugin as off, as the core does when it stops; its features stop first. */
  turnOff(): void {
    this.#on = false;
    for (const feature of this.#features.values()) {
      feature.followPlugin();
    }
    this.#followers.tellAll();
  }

  /** Reports the plugin as on, as the core does once it started; then its features that are on. */
  turnOn(): void {
    this.#on = true;
    this.#followers.tellAll();
    for (const feature of this.#features.values()) {
      feature.followPlugin();
    }
  }
}
