// The registry behind the shell's slots. The composition root creates one,
// registers the shell's own entries and every feature's entries, and provides
// it to the components through context.
import { createContext } from 'svelte';

import type {
  NavigationEntry,
  Overlay,
  PartRenderer,
  SettingsSection,
  ShellCommand,
  ShellView,
  SidebarPanel,
} from '#lib/types/shell-slots.js';

interface SlotEntry {
  readonly id: string;
}

/** The entries of one slot. Registering an ID twice is a bug and throws. */
export class Slot<T extends SlotEntry> {
  readonly #entries: T[] = $state([]);
  readonly #name: string;

  constructor(name: string) {
    this.#name = name;
  }

  /** The entries in the order they were registered. */
  get entries(): readonly T[] {
    return this.#entries;
  }

  /** The entry with this ID, if one is registered. */
  find(id: string): T | undefined {
    return this.#entries.find((entry) => entry.id === id);
  }

  /** Adds an entry. */
  register(entry: T): void {
    const isDuplicate = this.#entries.some((existing) => existing.id === entry.id);
    if (isDuplicate) {
      throw new Error(`The ${this.#name} slot already has an entry with the ID ${entry.id}.`);
    }
    this.#entries.push(entry);
  }
}

/** Every slot of the shell. */
export class ShellSlots {
  readonly commands = new Slot<ShellCommand>('commands');
  readonly navigation = new Slot<NavigationEntry>('navigation');
  readonly overlays = new Slot<Overlay>('overlays');
  readonly partRenderers = new Slot<PartRenderer>('part renderers');
  readonly settingsSections = new Slot<SettingsSection>('settings sections');
  readonly sidebarPanels = new Slot<SidebarPanel>('sidebar panels');
  readonly views = new Slot<ShellView>('views');
}

/** Reads and provides the shell's slots through Svelte's context. */
export const [getShellSlots, setShellSlots] = createContext<ShellSlots>();
