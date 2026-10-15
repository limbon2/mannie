// The side panels beside the main view (ADR-0025). Which panels are open
// comes from the main view's address (`utils/side-panels.ts`); this module
// opens, shows and closes them by changing the address, and keeps what the
// address does not hold: the tabs' labels, the controls that opened panels,
// and the panel whose heading takes the focus once it shows.
import { tick } from 'svelte';
import { SvelteMap } from 'svelte/reactivity';

import { goto } from '$app/navigation';
import { resolve } from '$app/paths';
import { page } from '$app/state';

import {
  openPanel,
  readSidePanels,
  type SidePanelState,
  withPanelClosed,
  withPanelOpened,
  withPanelShown,
  withSidePanels,
} from '#lib/utils/side-panels.js';

/** The ID of the main area, where the focus goes when a panel closes without its opener. */
export const MAIN_AREA_ID = 'main';

/** The label of a panel's tab. */
export interface PanelLabel {
  /** Whether the text is content, such as a conversation's title, rather than text from the catalog. */
  readonly isContent: boolean;
  /** The text. */
  readonly text: string;
}

/** How a panel opens. */
export interface OpenPanelOptions {
  /** A label to show until the panel names itself, such as a task's number and title. */
  readonly label?: PanelLabel;
  /** The control that opened the panel, where the focus returns when it closes. */
  readonly opener?: HTMLElement | null;
}

/** How a panel closes. */
export interface ClosePanelOptions {
  /**
   * Whether the focus returns to the panel's opener, or to the main area;
   * true unless the caller moves the focus itself.
   */
  readonly returnFocus?: boolean;
}

/** No panels: what the shell shows outside the main view, such as on the settings page. */
const NO_PANELS: SidePanelState = { panels: [], shown: null };

/** The side panels of the window. */
export class SidePanels {
  /**
   * The panel whose heading takes the focus once it shows: one that a
   * control opened. A reload shows panels without moving the focus.
   */
  focusRequest: string | null = $state(null);
  readonly #labels = new SvelteMap<string, PanelLabel>();
  readonly #openers = new SvelteMap<string, HTMLElement>();

  /** The open panels and the visible one, as the main view's address holds them. */
  get state(): SidePanelState {
    return page.route.id === '/' ? readSidePanels(page.url) : NO_PANELS;
  }

  /** Closes the panel `key`, and returns the focus to its opener or to the main area unless asked not to. */
  async close(key: string, options: ClosePanelOptions = {}): Promise<void> {
    const { returnFocus = true } = options;
    const opener = this.#openers.get(key);
    if (this.focusRequest === key) {
      this.focusRequest = null;
    }
    await this.#navigate(withPanelClosed(this.state, key));
    if (!returnFocus) {
      return;
    }
    await tick();
    const target = opener?.isConnected === true ? opener : document.getElementById(MAIN_AREA_ID);
    target?.focus();
  }

  /** Closes the visible panel, if one is open. */
  async closeShown(): Promise<void> {
    const shown = this.state.shown;
    if (shown !== null) {
      await this.close(shown);
    }
  }

  /** The label of the panel `key`'s tab, if the opener or the panel gave one. */
  label(key: string): PanelLabel | undefined {
    return this.#labels.get(key);
  }

  /**
   * Opens a panel of the kind `kind` that shows `argument`, or shows it when
   * it is open already, and moves the focus to it.
   */
  async open(kind: string, argument: string, options: OpenPanelOptions = {}): Promise<void> {
    const panel = openPanel(kind, argument);
    const { label, opener } = options;
    // A panel that named itself keeps its name over the opener's hint.
    const hasHint = label !== undefined;
    const isNamed = this.#labels.has(panel.key);
    if (hasHint && !isNamed) {
      this.#labels.set(panel.key, label);
    }
    if (opener) {
      this.#openers.set(panel.key, opener);
    }
    this.focusRequest = panel.key;
    await this.#navigate(withPanelOpened(this.state, panel));
  }

  /** Names the tab of the panel `key`, once the panel knows what it shows. */
  setLabel(key: string, label: PanelLabel): void {
    const current = this.#labels.get(key);
    const changed = current?.text !== label.text || current.isContent !== label.isContent;
    if (changed) {
      this.#labels.set(key, label);
    }
  }

  /** Shows the open panel `key`. */
  async show(key: string): Promise<void> {
    await this.#navigate(withPanelShown(this.state, key));
  }

  /** Changes the address to hold `next`, and forgets what belonged to panels that closed. */
  async #navigate(next: SidePanelState): Promise<void> {
    const open = next.panels.map((panel) => panel.key);
    for (const key of [...this.#labels.keys()]) {
      if (!open.includes(key)) {
        this.#labels.delete(key);
      }
    }
    for (const key of [...this.#openers.keys()]) {
      if (!open.includes(key)) {
        this.#openers.delete(key);
      }
    }
    const search = withSidePanels(page.url.search, next).toString();
    const query = search === '' ? '' : `?${search}`;
    // Panels replace the address in the history, so going back leaves the
    // view instead of closing its panels one by one, and the scroll
    // positions and the focus stay as they are.
    // eslint-disable-next-line svelte/no-navigation-without-resolve -- the path is resolve('/'); only the search parameters change, which resolve() does not take
    await goto(`${resolve('/')}${query}`, { replace: true, reset: false });
  }
}

/** The window's side panels. */
export const sidePanels = new SidePanels();
