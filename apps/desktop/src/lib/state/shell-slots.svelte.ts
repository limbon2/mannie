// The registry behind the shell's slots. The composition root creates one,
// registers the shell's own entries, every feature's and every plugin's,
// and provides it to the components through context.
//
// A plugin registers through `forPlugin`, which gives each of its entries
// the plugin's ID, and a slot lists such an entry only while the core
// reports the plugin as on (ADR-0030, decision 11). An optional feature of a
// plugin registers through `forFeature`, whose entries show only while the
// core reports the plugin and the feature as on (ADR-0055, decision 10). The
// shell changes when a plugin or a feature turns on or off, without a
// reload. Both classes hold the slots of the SDK's slot catalog, which the
// type checks compare (ADR-0034, decision 11).
import { createContext } from 'svelte';

import type { PluginSlots as SdkPluginSlots, SlotCatalog } from '#lib/sdk/slots.js';
import type {
  CommandKey,
  ContextKind,
  ConversationControl,
  ImageSourceKind,
  MentionKind,
  NavigationEntry,
  Overlay,
  PartRenderer,
  SettingsSection,
  ShellCommand,
  ShellView,
  SidebarPanel,
  SidePanelKind,
  SlotOwner,
  TaskKind,
  ToolKind,
} from '#lib/types/shell-slots.js';

interface SlotEntry {
  readonly id: string;
}

/** Whether the core reports the owner of entries as on: the plugin, and the feature of a feature's entries. */
export type OwnerIsOn = (owner: SlotOwner) => boolean;

interface Registered<T> {
  readonly entry: T;
  /** The plugin or feature that registered the entry, or `null` for the core's. */
  readonly owner: SlotOwner | null;
}

/**
 * The entries of one slot. Registering an ID twice is a bug and throws, as
 * does an entry that the slot's check refuses.
 */
export class Slot<T extends SlotEntry> {
  readonly #check: ((entry: T, registered: readonly T[]) => string | null) | undefined;
  readonly #isOn: OwnerIsOn;
  readonly #name: string;
  readonly #registered: Registered<T>[] = $state([]);

  /**
   * A slot. `isOn` tells whether the entries of a plugin or a feature show;
   * `check` returns why an entry cannot be registered beside those
   * registered before, or null.
   */
  constructor(name: string, isOn: OwnerIsOn, check?: (entry: T, registered: readonly T[]) => string | null) {
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
   * The entry with this ID when the plugin `plugin` or one of its features
   * registered it and it shows, so that an entry describes only what its own
   * plugin made, such as the calls of its tools (ADR-0039, decision 1).
   */
  findOf(id: string, plugin: string): T | undefined {
    const registered = this.#registered.find((candidate) => {
      const isEntry = candidate.entry.id === id;
      const isTheirs = candidate.owner?.plugin === plugin;
      return isEntry && isTheirs;
    });
    if (registered === undefined) {
      return undefined;
    }
    return this.#shows(registered) ? registered.entry : undefined;
  }

  /**
   * The entries that the plugin `plugin` or one of its features registered
   * and that show, in the order they were registered.
   */
  entriesOf(plugin: string): readonly T[] {
    return this.#registered
      .filter((registered) => {
        const isTheirs = registered.owner?.plugin === plugin;
        const shows = this.#shows(registered);
        return isTheirs && shows;
      })
      .map((registered) => registered.entry);
  }

  /**
   * The plugin or feature of the entry with this ID while it is off and the
   * entry does not show, so that an address of it can say so; otherwise
   * `null`.
   */
  ownerOff(id: string): SlotOwner | null {
    const registered = this.#registered.find((candidate) => candidate.entry.id === id);
    if (registered === undefined) {
      return null;
    }
    // The core's entries always show, so an entry that does not has an owner.
    return this.#shows(registered) ? null : registered.owner;
  }

  /** Adds an entry of the core's, or of the plugin or feature `owner`. */
  register(entry: T, owner: SlotOwner | null = null): void {
    const isDuplicate = this.#registered.some((existing) => existing.entry.id === entry.id);
    if (isDuplicate) {
      throw new Error(`The ${this.#name} slot already has an entry with the ID ${entry.id}.`);
    }
    const problem =
      this.#check?.(
        entry,
        this.#registered.map((registered) => registered.entry),
      ) ?? null;
    if (problem !== null) {
      throw new Error(problem);
    }
    this.#registered.push({ entry, owner });
  }

  #shows(registered: Registered<T>): boolean {
    if (registered.owner === null) {
      return true;
    }
    return this.#isOn(registered.owner);
  }
}

/** A slot as a plugin or one of its features registers in it: each entry belongs to it. */
export class PluginSlot<T extends SlotEntry> {
  readonly #owner: SlotOwner;
  readonly #slot: Slot<T>;

  constructor(slot: Slot<T>, owner: SlotOwner) {
    this.#slot = slot;
    this.#owner = owner;
  }

  /** Adds an entry of the plugin or the feature. */
  register(entry: T): void {
    this.#slot.register(entry, this.#owner);
  }
}

/** Why a side panel kind cannot be registered: the address separates its ID from the argument with a colon. */
function checkSidePanelKind(kind: SidePanelKind): string | null {
  return kind.id.includes(':') ? `The side panel kind ${kind.id} has a colon, which its address uses.` : null;
}

/**
 * The keys that the shell binds itself, by `KeyboardEvent.code`, with Ctrl
 * or Cmd: the palette, the settings and the sidebar.
 */
export const SHELL_KEYS: readonly string[] = ['KeyK', 'Comma', 'KeyB'];

/** Whether two keys of commands are the same key. */
function sameKey(left: CommandKey, right: CommandKey): boolean {
  const sameCode = left.code === right.code;
  const sameShift = (left.shift ?? false) === (right.shift ?? false);
  const sameAlt = (left.alt ?? false) === (right.alt ?? false);
  return sameCode && sameShift && sameAlt;
}

/** Whether a command has this key. */
function holdsKey(command: ShellCommand, key: CommandKey): boolean {
  if (command.key === undefined) {
    return false;
  }
  return sameKey(command.key, key);
}

/** Why a command cannot be registered: its key is the shell's or another command's (ADR-0053, decision 8). */
function checkCommand(command: ShellCommand, registered: readonly ShellCommand[]): string | null {
  const { key } = command;
  if (key === undefined) {
    return null;
  }
  const isShellKey = SHELL_KEYS.includes(key.code) && key.shift !== true && key.alt !== true;
  if (isShellKey) {
    return `The command ${command.id} takes the key ${key.code}, which the shell binds.`;
  }
  const holder = registered.find((other) => holdsKey(other, key));
  return holder === undefined ? null : `The command ${command.id} takes the key of the command ${holder.id}.`;
}

/** Every slot of the slot catalog, as the shell holds it. */
type CatalogSlots = { readonly [Name in keyof SlotCatalog]: Slot<SlotCatalog[Name]> };

/** Every slot of the shell. */
export class ShellSlots implements CatalogSlots {
  readonly commands: Slot<ShellCommand>;
  readonly contextKinds: Slot<ContextKind>;
  readonly conversationControls: Slot<ConversationControl>;
  readonly imageSources: Slot<ImageSourceKind>;
  readonly mentionKinds: Slot<MentionKind>;
  readonly navigation: Slot<NavigationEntry>;
  readonly overlays: Slot<Overlay>;
  readonly partRenderers: Slot<PartRenderer>;
  readonly settingsSections: Slot<SettingsSection>;
  readonly sidebarPanels: Slot<SidebarPanel>;
  readonly sidePanels: Slot<SidePanelKind>;
  readonly taskKinds: Slot<TaskKind>;
  readonly toolKinds: Slot<ToolKind>;
  readonly views: Slot<ShellView>;

  /**
   * The slots; `isOn` tells whether the entries of a plugin or a feature
   * show, and every plugin's and feature's do without it.
   */
  constructor(isOn: OwnerIsOn = () => true) {
    this.commands = new Slot('commands', isOn, checkCommand);
    this.contextKinds = new Slot('context kinds', isOn);
    this.conversationControls = new Slot('conversation controls', isOn);
    this.imageSources = new Slot('image sources', isOn);
    this.mentionKinds = new Slot('mention kinds', isOn);
    this.navigation = new Slot('navigation', isOn);
    this.overlays = new Slot('overlays', isOn);
    this.partRenderers = new Slot('part renderers', isOn);
    this.settingsSections = new Slot('settings sections', isOn);
    this.sidebarPanels = new Slot('sidebar panels', isOn);
    this.sidePanels = new Slot('side panels', isOn, checkSidePanelKind);
    this.taskKinds = new Slot('task kinds', isOn);
    this.toolKinds = new Slot('tool kinds', isOn);
    this.views = new Slot('views', isOn);
  }

  /** The slots as the plugin `plugin` registers in them. */
  forPlugin(plugin: string): PluginSlots {
    return new PluginSlots(this, { feature: null, plugin });
  }

  /** The slots as the optional feature `feature` of the plugin `plugin` registers in them. */
  forFeature(plugin: string, feature: string): PluginSlots {
    return new PluginSlots(this, { feature, plugin });
  }
}

/**
 * Every slot of the shell as one plugin, or one of its features, registers
 * in it (ADR-0030, decision 11; ADR-0055, decision 10).
 */
export class PluginSlots implements SdkPluginSlots {
  readonly commands: PluginSlot<ShellCommand>;
  readonly contextKinds: PluginSlot<ContextKind>;
  readonly conversationControls: PluginSlot<ConversationControl>;
  readonly imageSources: PluginSlot<ImageSourceKind>;
  readonly mentionKinds: PluginSlot<MentionKind>;
  readonly navigation: PluginSlot<NavigationEntry>;
  readonly overlays: PluginSlot<Overlay>;
  readonly partRenderers: PluginSlot<PartRenderer>;
  readonly settingsSections: PluginSlot<SettingsSection>;
  readonly sidebarPanels: PluginSlot<SidebarPanel>;
  readonly sidePanels: PluginSlot<SidePanelKind>;
  readonly taskKinds: PluginSlot<TaskKind>;
  readonly toolKinds: PluginSlot<ToolKind>;
  readonly views: PluginSlot<ShellView>;

  constructor(slots: ShellSlots, owner: SlotOwner) {
    this.commands = new PluginSlot(slots.commands, owner);
    this.contextKinds = new PluginSlot(slots.contextKinds, owner);
    this.conversationControls = new PluginSlot(slots.conversationControls, owner);
    this.imageSources = new PluginSlot(slots.imageSources, owner);
    this.mentionKinds = new PluginSlot(slots.mentionKinds, owner);
    this.navigation = new PluginSlot(slots.navigation, owner);
    this.overlays = new PluginSlot(slots.overlays, owner);
    this.partRenderers = new PluginSlot(slots.partRenderers, owner);
    this.settingsSections = new PluginSlot(slots.settingsSections, owner);
    this.sidebarPanels = new PluginSlot(slots.sidebarPanels, owner);
    this.sidePanels = new PluginSlot(slots.sidePanels, owner);
    this.taskKinds = new PluginSlot(slots.taskKinds, owner);
    this.toolKinds = new PluginSlot(slots.toolKinds, owner);
    this.views = new PluginSlot(slots.views, owner);
  }
}

/** Reads and provides the shell's slots through Svelte's context. */
export const [getShellSlots, setShellSlots] = createContext<ShellSlots>();
