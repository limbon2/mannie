// The registry behind the shell's slots. The composition root creates one,
// registers the shell's own entries and every feature's entries, and provides
// it to the components through context.
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
} from '#lib/types/shell-slots.js';

interface SlotEntry {
  readonly id: string;
}

/**
 * The entries of one slot. Registering an ID twice is a bug and throws, as
 * does an entry that the slot's check refuses.
 */
export class Slot<T extends SlotEntry> {
  readonly #check: ((entry: T) => string | null) | undefined;
  readonly #entries: T[] = $state([]);
  readonly #name: string;

  /** A slot; `check` returns why an entry cannot be registered, or null. */
  constructor(name: string, check?: (entry: T) => string | null) {
    this.#name = name;
    this.#check = check;
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
    const problem = this.#check?.(entry) ?? null;
    if (problem !== null) {
      throw new Error(problem);
    }
    this.#entries.push(entry);
  }
}

/** Why a side panel kind cannot be registered: the address separates its ID from the argument with a colon. */
function checkSidePanelKind(kind: SidePanelKind): string | null {
  return kind.id.includes(':') ? `The side panel kind ${kind.id} has a colon, which its address uses.` : null;
}

/** Every slot of the shell. */
export class ShellSlots {
  readonly commands = new Slot<ShellCommand>('commands');
  readonly conversationControls = new Slot<ConversationControl>('conversation controls');
  readonly navigation = new Slot<NavigationEntry>('navigation');
  readonly overlays = new Slot<Overlay>('overlays');
  readonly partRenderers = new Slot<PartRenderer>('part renderers');
  readonly settingsSections = new Slot<SettingsSection>('settings sections');
  readonly sidebarPanels = new Slot<SidebarPanel>('sidebar panels');
  readonly sidePanels = new Slot<SidePanelKind>('side panels', checkSidePanelKind);
  readonly views = new Slot<ShellView>('views');
}

/** Reads and provides the shell's slots through Svelte's context. */
export const [getShellSlots, setShellSlots] = createContext<ShellSlots>();
