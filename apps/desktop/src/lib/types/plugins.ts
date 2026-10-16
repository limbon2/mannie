// The UI side of an internal plugin (ADR-0030, decision 11): what the
// composition root lists for each plugin, besides the entries that the
// plugin registers in the shell's slots.
import type { PluginWork } from '#lib/generated/bindings.js';
import type { PluginSlots } from '#lib/state/shell-slots.svelte.js';

import type { Message } from './shell-slots.js';

/** A plugin as the UI knows it. Its name and description are messages under its key prefix. */
export interface PluginRegistration {
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
   * switch asks about first, or `null` when that work is none.
   */
  readonly offQuestion: (work: PluginWork) => string | null;
  /** Registers the plugin's entries, which show only while the plugin is on. */
  readonly register: (slots: PluginSlots) => void;
}

/** What the UI shows of a plugin wherever the plugin's own code does not run, such as the Plugins page. */
export type PluginDescriptor = Omit<PluginRegistration, 'register'>;
