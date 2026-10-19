// The plugins of a build as the UI knows them (ADR-0035, decision 7): the
// generated plugin list names each plugin with the messages of its catalog
// and carries the definition that the plugin's UI exports, if it has a UI
// (ADR-0034, decision 11).
import type { WorkItem } from '#lib/generated/bindings.js';
import type { PluginDefinition } from '#lib/sdk/index.js';

import type { Message } from './shell-slots.js';

/** A plugin of the generated plugin list. */
export interface PluginEntry {
  /** The plugin's UI, or `null` for a plugin without one. */
  readonly definition: PluginDefinition | null;
  /** What the plugin adds, in one sentence, for the Plugins page. */
  readonly description: Message;
  /** The plugin's ID, as the core reports it. */
  readonly id: string;
  /** The plugin's name. */
  readonly name: Message;
}

/** What the UI shows of a plugin wherever the plugin's own code does not run, such as the Plugins page. */
export interface PluginDescriptor {
  /** What the plugin adds, in one sentence, for the Plugins page. */
  readonly description: Message;
  /** The sentence of one of the plugin's start failure codes, or `null` for a code it does not know. */
  readonly failure: (code: string) => string | null;
  /** The plugin's ID, as the core reports it. */
  readonly id: string;
  /** The plugin's name. */
  readonly name: Message;
  /**
   * What turning the plugin off does to its work in progress, which the
   * switch asks about first, or `null` when there is none. `items` holds
   * each kind of work in progress by the plugin's code.
   */
  readonly offQuestion: (items: readonly WorkItem[]) => string | null;
}
