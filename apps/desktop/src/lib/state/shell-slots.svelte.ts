// The registry behind the shell's slots. The composition root creates one,
// registers the shell's own entries, every feature's and every plugin's,
// and provides it to the components through context.
//
// A plugin registers through `forPlugin`, which gives each of its entries
// the plugin's ID, and a slot lists such an entry only while the core
// reports the plugin as on (ADR-0030, decision 11). The shell changes when a
// plugin turns on or off, without a reload.
import { createContext } from 'svelte';

import type {
  ConversationControl,
  NavigationEntry,
  Overlay,
  PartRenderer,
  SettingsSection,
  ShellCommand,
  ShellView,
  SidebarPanel,
  SidePanelKind,
  TaskKind,
} from '#lib/types/shell-slots.js';

interface SlotEntry {
  readonly id: string;
}

/** Whether the core reports a plugin as on. */
export type PluginIsOn = (plugin: string) => boolean;

interface Registered<T> {
  readonly entry: T;
  /** The plugin that registered the entry, or `null` for the core's. */
  readonly plugin: string | null;
}

/**
 * The entries of one slot. Registering an ID twice is a bug and throws, as
 * does an entry that the slot's check refuses.
 */
export class Slot<T extends SlotEntry> {
  readonly #check: ((entry: T) => string | null) | undefined;
  readonly #isOn: PluginIsOn;
  readonly #name: string;
  readonly #registered: Registered<T>[] = $state([]);

  /**
   * A slot. `isOn` tells whether a plugin's entries show; `check` returns why
   * an entry cannot be registered, or null.
   */
  constructor(name: string, isOn: PluginIsOn, check?: (entry: T) => string | null) {
    this.#name = name;
    this.#isOn = isOn;
    this.#check = check;
  }

  /** The entries that show, in the order they were registered. */
  get entries(): readonly T[] {
    return this.#registered.filter((registered) => this.#shows(registered)).map((registered) => registered.entry);
  }

  /** The entry with this ID, if one is registered and shows. */
  find(id: string): T | undefined {
    const registered = this.#registered.find((candidate) => candidate.entry.id === id);
    if (registered === undefined) {
      return undefined;
    }
    return this.#shows(registered) ? registered.entry : undefined;
  }

  /**
   * The plugin of the entry with this ID while the plugin is off and the
   * entry does not show, so that an address of it can say so; otherwise
   * `null`.
   */
  pluginOff(id: string): string | null {
    const registered = this.#registered.find((candidate) => candidate.entry.id === id);
    if (registered === undefined) {
      return null;
    }
    // The core's entries always show, so an entry that does not is a plugin's.
    return this.#shows(registered) ? null : registered.plugin;
  }

  /** Adds an entry of the core's, or of the plugin `plugin`. */
  register(entry: T, plugin: string | null = null): void {
    const isDuplicate = this.#registered.some((existing) => existing.entry.id === entry.id);
    if (isDuplicate) {
      throw new Error(`The ${this.#name} slot already has an entry with the ID ${entry.id}.`);
    }
    const problem = this.#check?.(entry) ?? null;
    if (problem !== null) {
      throw new Error(problem);
    }
    this.#registered.push({ entry, plugin });
  }

  #shows(registered: Registered<T>): boolean {
    if (registered.plugin === null) {
      return true;
    }
    return this.#isOn(registered.plugin);
  }
}

/** A slot as a plugin registers in it: each entry belongs to the plugin. */
export class PluginSlot<T extends SlotEntry> {
  readonly #plugin: string;
  readonly #slot: Slot<T>;

  constructor(slot: Slot<T>, plugin: string) {
    this.#slot = slot;
    this.#plugin = plugin;
  }

  /** Adds an entry of the plugin. */
  register(entry: T): void {
    this.#slot.register(entry, this.#plugin);
  }
}

/** Why a side panel kind cannot be registered: the address separates its ID from the argument with a colon. */
function checkSidePanelKind(kind: SidePanelKind): string | null {
  return kind.id.includes(':') ? `The side panel kind ${kind.id} has a colon, which its address uses.` : null;
}

/** Every slot of the shell. */
export class ShellSlots {
  readonly commands: Slot<ShellCommand>;
  readonly conversationControls: Slot<ConversationControl>;
  readonly navigation: Slot<NavigationEntry>;
  readonly overlays: Slot<Overlay>;
  readonly partRenderers: Slot<PartRenderer>;
  readonly settingsSections: Slot<SettingsSection>;
  readonly sidebarPanels: Slot<SidebarPanel>;
  readonly sidePanels: Slot<SidePanelKind>;
  readonly taskKinds: Slot<TaskKind>;
  readonly views: Slot<ShellView>;

  /** The slots; `isOn` tells whether a plugin's entries show, and every plugin's do without it. */
  constructor(isOn: PluginIsOn = () => true) {
    this.commands = new Slot('commands', isOn);
    this.conversationControls = new Slot('conversation controls', isOn);
    this.navigation = new Slot('navigation', isOn);
    this.overlays = new Slot('overlays', isOn);
    this.partRenderers = new Slot('part renderers', isOn);
    this.settingsSections = new Slot('settings sections', isOn);
    this.sidebarPanels = new Slot('sidebar panels', isOn);
    this.sidePanels = new Slot('side panels', isOn, checkSidePanelKind);
    this.taskKinds = new Slot('task kinds', isOn);
    this.views = new Slot('views', isOn);
  }

  /** The slots as the plugin `plugin` registers in them. */
  forPlugin(plugin: string): PluginSlots {
    return new PluginSlots(this, plugin);
  }
}

/** Every slot of the shell as one plugin registers in it (ADR-0030, decision 11). */
export class PluginSlots {
  readonly commands: PluginSlot<ShellCommand>;
  readonly conversationControls: PluginSlot<ConversationControl>;
  readonly navigation: PluginSlot<NavigationEntry>;
  readonly overlays: PluginSlot<Overlay>;
  readonly partRenderers: PluginSlot<PartRenderer>;
  readonly settingsSections: PluginSlot<SettingsSection>;
  readonly sidebarPanels: PluginSlot<SidebarPanel>;
  readonly sidePanels: PluginSlot<SidePanelKind>;
  readonly taskKinds: PluginSlot<TaskKind>;
  readonly views: PluginSlot<ShellView>;

  constructor(slots: ShellSlots, plugin: string) {
    this.commands = new PluginSlot(slots.commands, plugin);
    this.conversationControls = new PluginSlot(slots.conversationControls, plugin);
    this.navigation = new PluginSlot(slots.navigation, plugin);
    this.overlays = new PluginSlot(slots.overlays, plugin);
    this.partRenderers = new PluginSlot(slots.partRenderers, plugin);
    this.settingsSections = new PluginSlot(slots.settingsSections, plugin);
    this.sidebarPanels = new PluginSlot(slots.sidebarPanels, plugin);
    this.sidePanels = new PluginSlot(slots.sidePanels, plugin);
    this.taskKinds = new PluginSlot(slots.taskKinds, plugin);
    this.views = new PluginSlot(slots.views, plugin);
  }
}

/** Reads and provides the shell's slots through Svelte's context. */
export const [getShellSlots, setShellSlots] = createContext<ShellSlots>();
