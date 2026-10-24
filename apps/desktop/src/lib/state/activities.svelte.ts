// The activity bar's selection (ADR-0045, decision 4): which navigation entry
// the bar selects, and the address that the main area last had under each
// entry, so that selecting an entry shows the place where the owner left it.
// Navigating inside the main area keeps the selection. It is a convenience of
// one window, kept in the webview's storage as the sidebar's collapsed parts
// are; without the storage, the entry of the shown view is selected at each
// start.
import { goto } from '$app/navigation';
import { resolve } from '$app/paths';

import type { NavigationEntry } from '#lib/types/shell-slots.js';
import { sortedByOrder } from '#lib/utils/order.js';
import { viewSearch } from '#lib/utils/views.js';

import { browserStorage } from './sidebar-collapse.svelte.js';

/** The key of the stored state. */
export const ACTIVITY_STORAGE_KEY = 'mannie.shell.activity';

/** What the storage holds. */
interface Stored {
  /** The search of the main area's address under each entry, with its `?`, by the entry's ID. */
  readonly addresses: Readonly<Partial<Record<string, string>>>;
  /** The selected entry's ID, or `null` before the owner selected one. */
  readonly selected: string | null;
}

function isStored(value: unknown): value is Stored {
  const isObject = typeof value === 'object' && value !== null;
  if (!isObject) {
    return false;
  }
  const { addresses, selected } = value as Record<string, unknown>;
  const hasSelection = selected === null || typeof selected === 'string';
  const isRecord = typeof addresses === 'object' && addresses !== null && !Array.isArray(addresses);
  const hasAddresses = isRecord && Object.values(addresses).every((address) => typeof address === 'string');
  return hasSelection && hasAddresses;
}

/** The navigation entries that the bar shows; the shell gives its navigation slot. */
export interface ActivityEntries {
  readonly entries: readonly NavigationEntry[];
}

/** How a navigation of the main area goes. */
export interface NavigateOptions {
  /** Whether the address replaces the current one in the history, so that Back skips it. */
  readonly replace?: boolean;
}

/** Shows an address of the main area: the search of `/`, with its `?`, or empty. */
export type Navigate = (search: string, options?: NavigateOptions) => Promise<void>;

/** The bar's selection and the addresses that it keeps. */
export class Activities {
  /** The ID of the selected entry, or `null` before the owner selected one. */
  selected: string | null = $state(null);
  #addresses: Partial<Record<string, string>> = $state({});
  #navigation: ActivityEntries = { entries: [] };
  readonly #navigate: Navigate;
  readonly #storage: Storage | null;

  constructor(storage: Storage | null, navigate: Navigate) {
    this.#storage = storage;
    this.#navigate = navigate;
    const stored = this.#read();
    if (stored !== null) {
      this.selected = stored.selected;
      this.#addresses = { ...stored.addresses };
    }
  }

  /** The entries in the bar's order. */
  get entries(): NavigationEntry[] {
    return sortedByOrder(this.#navigation.entries);
  }

  /** The entry with the lowest order, which the sidebar panels that name no entry belong to. */
  get first(): NavigationEntry | undefined {
    return this.entries.at(0);
  }

  /** Where selecting an entry leads: the address that the main area last had under it, or its view. */
  addressOf(entry: NavigationEntry): string {
    return this.#addresses[entry.id] ?? viewSearch(entry.view);
  }

  /** Follows the entries of the shell's navigation slot. */
  attach(navigation: ActivityEntries): void {
    this.#navigation = navigation;
  }

  /**
   * The entry that the bar shows as selected: the selected one while it
   * shows, none while it does not, as while its plugin starts, and before the
   * owner selected one, the entry of the shown view or else the first.
   */
  current(shownView: string | null): NavigationEntry | undefined {
    const entries = this.entries;
    if (this.selected !== null) {
      return entries.find((entry) => entry.id === this.selected);
    }
    return entries.find((entry) => entry.view === shownView) ?? entries.at(0);
  }

  /** Selects the first entry again, with its address, as when the selected entry's plugin stopped. */
  async fallBack(): Promise<void> {
    const first = this.first;
    if (first !== undefined) {
      await this.select(first);
    }
  }

  /**
   * Selects the first entry again without showing it, as when the selected
   * entry's plugin stopped while the settings are open, and returns the
   * address that the settings then lead back to, or `null` without entries.
   */
  fallBackInPlace(): string | null {
    const first = this.first;
    if (first === undefined) {
      return null;
    }
    this.#choose(first.id);
    return this.addressOf(first);
  }

  /**
   * Shows a view with its parameters, and selects the view's entry when it
   * has one. With `replace`, the address replaces the current one in the
   * history, such as a view's bare address that leads to one of its things.
   */
  async openView(
    view: string,
    parameters: Readonly<Record<string, string>> = {},
    options: NavigateOptions = {},
  ): Promise<void> {
    const entry = this.entries.find((candidate) => candidate.view === view);
    if (entry !== undefined) {
      this.#choose(entry.id);
    }
    await this.#navigate(viewSearch(view, parameters), options);
  }

  /** Whether a sidebar panel that names `activity`, or none, belongs to the entry. */
  owns(entry: NavigationEntry, activity: string | undefined): boolean {
    const owner = activity ?? this.first?.id;
    return owner === entry.id;
  }

  /** Keeps the main area's address, its search with the `?`, for the entry that is selected now. */
  remember(entry: NavigationEntry, search: string): void {
    if (this.#addresses[entry.id] === search) {
      return;
    }
    this.#addresses = { ...this.#addresses, [entry.id]: search };
    this.#save();
  }

  /** Selects an entry and shows the address that the main area last had under it. */
  async select(entry: NavigationEntry): Promise<void> {
    this.#choose(entry.id);
    await this.#navigate(this.addressOf(entry));
  }

  #choose(id: string): void {
    if (this.selected === id) {
      return;
    }
    this.selected = id;
    this.#save();
  }

  #read(): Stored | null {
    try {
      const text = this.#storage?.getItem(ACTIVITY_STORAGE_KEY) ?? null;
      if (text === null) {
        return null;
      }
      const value: unknown = JSON.parse(text);
      return isStored(value) ? value : null;
    } catch {
      // Unreadable storage or a stored value that is not JSON: the entry of
      // the shown view is selected.
      return null;
    }
  }

  #save(): void {
    const stored: Stored = { addresses: this.#addresses, selected: this.selected };
    try {
      this.#storage?.setItem(ACTIVITY_STORAGE_KEY, JSON.stringify(stored));
    } catch {
      // A full or blocked storage keeps the selection for this window only.
    }
  }
}

/** Shows an address of the main area at the app's root. */
async function navigateMainArea(search: string, options: NavigateOptions = {}): Promise<void> {
  // eslint-disable-next-line svelte/no-navigation-without-resolve -- the path is resolve('/'); only the search parameters change, which resolve() does not take
  await goto(`${resolve('/')}${search}`, { replaceState: options.replace === true });
}

/** The window's activity bar selection. */
export const activities = new Activities(browserStorage(), navigateMainArea);
