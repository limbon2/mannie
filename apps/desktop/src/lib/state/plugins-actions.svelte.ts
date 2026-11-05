// What the Plugins page does (docs/features/plugins.md): turning a plugin or
// one of its optional features on or off (ADR-0055, decision 10), asking
// first when turning one off would let work in progress go, and trying a
// failed start again.
import { SvelteMap, SvelteSet } from 'svelte/reactivity';

import type { PluginsError } from '#lib/generated/bindings.js';
import type { Failure } from '#lib/types/calls.js';
import { pluginOffQuestion } from '#lib/utils/plugins-messages.js';

import { pluginStates } from './plugin-states.svelte.js';

/** A plugin, or one of its features, whose turning off waits for the owner's answer. */
export interface OffQuestion {
  /** The plugin's feature that turns off, or `null` when the plugin does. */
  readonly feature: string | null;
  /** The plugin. */
  readonly id: string;
  /** What turning it off does to its work in progress. */
  readonly question: string;
}

/**
 * The key of a plugin's feature among the switches that are changing and
 * the failures. A plugin's ID has no slash, so it never equals one.
 */
export function featureKey(plugin: string, feature: string): string {
  return `${plugin}/${feature}`;
}

/** The Plugins page's actions. */
export class PluginsActions {
  /** The plugins, by ID, and the features, by `featureKey`, whose switch is changing. */
  readonly changing = new SvelteSet<string>();
  /** Why the last change of a plugin or a feature failed, by the same keys. */
  readonly failures = new SvelteMap<string, Failure<PluginsError>>();
  /** Whether the question before turning a plugin off shows. */
  offOpen = $state(false);
  /** The plugin whose turning off waits for the owner's answer. */
  offQuestion: OffQuestion | null = $state(null);

  /** Turns the plugin or the feature off after the owner confirmed. */
  async confirmOff(): Promise<void> {
    const question = this.offQuestion;
    this.offOpen = false;
    this.offQuestion = null;
    if (question === null) {
      return;
    }
    if (question.feature === null) {
      await this.#set(question.id, false);
      return;
    }
    await this.#setFeature(question.id, question.feature, false);
  }

  /** Tries the start of a failed plugin again. */
  async retry(id: string): Promise<void> {
    await this.#set(id, true);
  }

  /** Tries the start of a failed feature again. */
  async retryFeature(plugin: string, feature: string): Promise<void> {
    await this.#setFeature(plugin, feature, true);
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
    const question = work.ok ? pluginOffQuestion(id, work.data.items) : null;
    if (question !== null) {
      this.offQuestion = { feature: null, id, question };
      this.offOpen = true;
      return;
    }
    await this.#set(id, false);
  }

  /**
   * Turns a plugin's feature on, or off, asking first as for a plugin. The
   * plugin's UI words the feature's work, whose codes are the plugin's.
   */
  async toggleFeature(plugin: string, feature: string, enabled: boolean): Promise<void> {
    if (enabled) {
      await this.#setFeature(plugin, feature, true);
      return;
    }
    const work = await pluginStates.featureWork(plugin, feature);
    const question = work.ok ? pluginOffQuestion(plugin, work.data.items) : null;
    if (question !== null) {
      this.offQuestion = { feature, id: plugin, question };
      this.offOpen = true;
      return;
    }
    await this.#setFeature(plugin, feature, false);
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

  async #setFeature(plugin: string, feature: string, enabled: boolean): Promise<void> {
    const key = featureKey(plugin, feature);
    this.changing.add(key);
    this.failures.delete(key);
    const failure = await pluginStates.setFeatureEnabled(plugin, feature, enabled);
    this.changing.delete(key);
    if (failure !== null) {
      this.failures.set(key, failure);
    }
  }
}

/** The Plugins page's actions. */
export const pluginsActions = new PluginsActions();
