// What the Plugins page does (docs/features/plugins.md): turning a plugin on
// or off, asking first when turning one off would let work in progress go,
// and trying a failed start again.
import { SvelteMap, SvelteSet } from 'svelte/reactivity';

import type { PluginsError } from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';

import { findPlugin } from './plugin-catalog.js';
import { pluginStates } from './plugin-states.svelte.js';

/** A plugin whose turning off waits for the owner's answer. */
export interface OffQuestion {
  /** The plugin. */
  readonly id: string;
  /** What turning it off does to its work in progress. */
  readonly question: string;
}

/** The Plugins page's actions. */
export class PluginsActions {
  /** The plugins whose switch is changing. */
  readonly changing = new SvelteSet<string>();
  /** Why the last change of a plugin failed, by plugin. */
  readonly failures = new SvelteMap<string, Failure<PluginsError>>();
  /** Whether the question before turning a plugin off shows. */
  offOpen = $state(false);
  /** The plugin whose turning off waits for the owner's answer. */
  offQuestion: OffQuestion | null = $state(null);

  /** Turns the plugin off after the owner confirmed. */
  async confirmOff(): Promise<void> {
    const question = this.offQuestion;
    this.offOpen = false;
    this.offQuestion = null;
    if (question !== null) {
      await this.#set(question.id, false);
    }
  }

  /** Tries the start of a failed plugin again. */
  async retry(id: string): Promise<void> {
    await this.#set(id, true);
  }

  /**
   * Turns a plugin on, or off. Turning off a plugin with work in progress
   * asks first and says what happens to that work; work that cannot be read
   * asks nothing, since the plugin says what it leaves as it stops.
   */
  async toggle(id: string, enabled: boolean): Promise<void> {
    if (enabled) {
      await this.#set(id, true);
      return;
    }
    const work = await pluginStates.work(id);
    const question = work.ok ? (findPlugin(id)?.offQuestion(work.data.items) ?? null) : null;
    if (question !== null) {
      this.offQuestion = { id, question };
      this.offOpen = true;
      return;
    }
    await this.#set(id, false);
  }

  async #set(id: string, enabled: boolean): Promise<void> {
    this.changing.add(id);
    this.failures.delete(id);
    const failure = await pluginStates.setEnabled(id, enabled);
    this.changing.delete(id);
    if (failure !== null) {
      this.failures.set(id, failure);
    }
  }
}

/** The Plugins page's actions. */
export const pluginsActions = new PluginsActions();
