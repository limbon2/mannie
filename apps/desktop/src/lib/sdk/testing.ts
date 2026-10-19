// Test helpers of the TypeScript plugin SDK, `@mannie/plugin-sdk/testing`
// (ADR-0034, decision 11): a plugin's tests register its UI in a context of
// their own, with the shell's real slots, and turn the plugin on and off as
// the core would report it. Only tests import this module.
import { ShellSlots } from '#lib/state/shell-slots.svelte.js';

import type { PluginContext } from './index.js';
import type { PluginSlots } from './slots.js';

/** The images of a message box, the target of an image source's files. */
export { MessageImages } from '#lib/state/message-images.svelte.js';

/** Something that follows whether the plugin is on. */
interface Follower {
  readonly off: () => void;
  readonly on: () => void;
}

/** A plugin's context in a test, whose plugin is on only while the test says so. */
export class TestPluginContext implements PluginContext {
  readonly id: string;
  /** The shell's slots, where the test finds the plugin's entries. */
  readonly shell: ShellSlots;
  readonly slots: PluginSlots;
  #followers: Follower[] = [];
  #on: boolean;

  /** The context of the plugin `id`, which starts off unless `on` says otherwise. */
  constructor(id: string, on = false) {
    this.id = id;
    this.#on = on;
    this.shell = new ShellSlots((plugin) => {
      const isThisPlugin = plugin === id;
      return isThisPlugin && this.#on;
    });
    this.slots = this.shell.forPlugin(id);
  }

  readonly isOn = (): boolean => this.#on;

  readonly whileOn = (on: () => void, off: () => void): (() => void) => {
    const follower: Follower = { off, on };
    this.#followers.push(follower);
    if (this.#on) {
      on();
    }
    return () => {
      this.#followers = this.#followers.filter((other) => other !== follower);
    };
  };

  /** Reports the plugin as off, as the core does when it stops. */
  turnOff(): void {
    if (!this.#on) {
      return;
    }
    this.#on = false;
    for (const follower of this.#followers) {
      follower.off();
    }
  }

  /** Reports the plugin as on, as the core does once it started. */
  turnOn(): void {
    if (this.#on) {
      return;
    }
    this.#on = true;
    for (const follower of this.#followers) {
      follower.on();
    }
  }
}
