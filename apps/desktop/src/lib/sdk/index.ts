// The TypeScript plugin SDK, `@mannie/plugin-sdk` (ADR-0034, decision 11): a
// plugin's UI exports `definePlugin(...)` from its `ui/index.ts`, and the
// composition root calls its `register` with the plugin's context. A
// plugin's UI imports this SDK, its own files and its generated files, and
// nothing else (ADR-0035, decision 6). The other parts of the SDK are
// `@mannie/plugin-sdk/<part>`: `slots`, `ipc`, `messages`, `format`,
// `navigation`, `conversations`, `images`, `links`, `message-box`,
// `terminal`, `components`, `components/<name>` for the installed
// shadcn-svelte components, `icons/<name>` and `testing`.
import type { WorkItem } from '#lib/generated/bindings.js';

import type { PluginSlots } from './slots.js';

export type { WorkItem } from '#lib/generated/bindings.js';
export type { Message } from '#lib/types/shell-slots.js';

/**
 * What a plugin's UI gets for one of the plugin's optional features
 * (ADR-0055, decision 10): its entries show only while the plugin and the
 * feature are on.
 */
export interface FeatureContext {
  /** The feature's ID, as the plugin's manifest names it. */
  readonly id: string;
  /** Whether the core reports the plugin and the feature as on. */
  readonly isOn: () => boolean;
  /** The shell's slots, whose entries of this feature show only while the plugin and the feature are on. */
  readonly slots: PluginSlots;
  /**
   * Calls `on` now if the core reports the plugin and the feature as on and
   * each time they turn on later, and `off` each time they stop being on,
   * until the returned function stops following. A feature's state follows
   * the feature through it, so that its subscriptions run only while the
   * feature is on.
   */
  readonly whileOn: (on: () => void, off: () => void) => () => void;
}

/** What a plugin's UI gets when the composition root registers it. */
export interface PluginContext {
  /**
   * The context of one of the plugin's optional features. A feature that
   * the plugin's manifest does not list throws, as a bug of the plugin.
   */
  readonly feature: (id: string) => FeatureContext;
  /** The plugin's ID, which equals its folder's name. */
  readonly id: string;
  /** Whether the core reports the plugin as on. */
  readonly isOn: () => boolean;
  /** The shell's slots, whose entries of this plugin show only while the plugin is on. */
  readonly slots: PluginSlots;
  /**
   * Calls `on` now if the core reports the plugin as on and each time it
   * turns on later, and `off` each time it stops being on, until the
   * returned function stops following. A plugin's state follows its plugin
   * through it, so that its subscriptions run only while the plugin is on
   * (ADR-0030, decision 11).
   */
  readonly whileOn: (on: () => void, off: () => void) => () => void;
}

/** A plugin's UI: what the Plugins page asks of it, and the entries that it registers. */
export interface PluginDefinition {
  /** The sentence of one of the plugin's start failure codes, or `null` for a code it does not know. */
  readonly failure?: (code: string) => string | null;
  /**
   * What turning the plugin off does to its work in progress, which the
   * switch asks about first, or `null` when there is none. `items` holds
   * each kind of work in progress by the plugin's code.
   */
  readonly offQuestion?: (items: readonly WorkItem[]) => string | null;
  /** Registers the plugin's entries in the shell's slots. */
  readonly register: (context: PluginContext) => void;
}

/** A plugin's UI, which its `ui/index.ts` exports as its default export. */
export function definePlugin(definition: PluginDefinition): PluginDefinition {
  return definition;
}
