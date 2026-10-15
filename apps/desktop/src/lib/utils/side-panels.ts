// Which side panels are open (ADR-0025, decision 4). The main view's address
// holds them, so they belong to the view that opened them: a view that opens
// other content builds an address without them, the settings lead back to
// them, and a reload shows them again. Each `panel` search parameter names an
// open panel as `<kind>:<argument>`, in the order of the tabs, and
// `shown-panel` names the visible one.

/** The search parameter that names an open side panel, once for each panel. */
export const PANEL_PARAMETER = 'panel';

/** The search parameter that names the visible side panel. */
export const SHOWN_PANEL_PARAMETER = 'shown-panel';

/** The most side panels open at once; opening another closes the oldest. */
export const MAX_OPEN_PANELS = 8;

/** The longest panel address that is read; a longer one is ignored. */
const MAX_PANEL_ADDRESS_LENGTH = 512;

/** An open side panel. */
export interface OpenPanel {
  /** What the panel shows, such as a conversation's ID. */
  readonly argument: string;
  /** The panel's address, `<kind>:<argument>`, which tells the panel apart. */
  readonly key: string;
  /** The panel's kind. */
  readonly kind: string;
}

/** The open side panels, in the order of their tabs, and the visible one. */
export interface SidePanelState {
  /** The open panels. */
  readonly panels: readonly OpenPanel[];
  /** The key of the visible panel, or `null` when none is open. */
  readonly shown: string | null;
}

/** The part of a URL that holds the panels; SvelteKit's `page.url` is read-only. */
interface PanelUrl {
  readonly searchParams: { get(name: string): string | null; getAll(name: string): string[] };
}

/** A panel of a kind, with an argument. */
export function openPanel(kind: string, argument: string): OpenPanel {
  return { argument, key: `${kind}:${argument}`, kind };
}

/** The panel that an address names, or `null` for an address that names none. */
export function parsePanel(address: string): OpenPanel | null {
  if (address.length > MAX_PANEL_ADDRESS_LENGTH) {
    return null;
  }
  const separator = address.indexOf(':');
  const hasKind = separator > 0;
  const hasArgument = separator !== -1 && separator < address.length - 1;
  if (!hasKind || !hasArgument) {
    return null;
  }
  return openPanel(address.slice(0, separator), address.slice(separator + 1));
}

/**
 * The panels that a URL holds: the valid ones without repeats, the newest
 * if there are too many, and the visible one, which is the last tab when the
 * URL names none or a panel that is not open.
 */
export function readSidePanels(url: PanelUrl): SidePanelState {
  const panels: OpenPanel[] = [];
  for (const address of url.searchParams.getAll(PANEL_PARAMETER)) {
    const panel = parsePanel(address);
    if (panel === null) {
      continue;
    }
    const isRepeat = panels.some((open) => open.key === panel.key);
    if (!isRepeat) {
      panels.push(panel);
    }
  }
  const kept = panels.slice(-MAX_OPEN_PANELS);
  const requested = url.searchParams.get(SHOWN_PANEL_PARAMETER);
  const isRequestedOpen = kept.some((panel) => panel.key === requested);
  const shown = isRequestedOpen ? requested : (kept.at(-1)?.key ?? null);
  return { panels: kept, shown };
}

/** The search parameters of an address with these panels; every other parameter of `search` stays. */
export function withSidePanels(search: string, state: SidePanelState): URLSearchParams {
  const next = new URLSearchParams(search);
  next.delete(PANEL_PARAMETER);
  next.delete(SHOWN_PANEL_PARAMETER);
  for (const panel of state.panels) {
    next.append(PANEL_PARAMETER, panel.key);
  }
  if (state.shown !== null) {
    next.set(SHOWN_PANEL_PARAMETER, state.shown);
  }
  return next;
}

/**
 * The panels after opening `panel`: it shows, in a tab of its own at the end
 * when it is new, and the oldest panels close beyond the limit.
 */
export function withPanelOpened(state: SidePanelState, panel: OpenPanel): SidePanelState {
  const isOpen = state.panels.some((open) => open.key === panel.key);
  const panels = isOpen ? state.panels : [...state.panels, panel].slice(-MAX_OPEN_PANELS);
  return { panels, shown: panel.key };
}

/** The panels with the panel `key` shown, when it is open. */
export function withPanelShown(state: SidePanelState, key: string): SidePanelState {
  const isOpen = state.panels.some((panel) => panel.key === key);
  return isOpen ? { panels: state.panels, shown: key } : state;
}

/**
 * The panels after closing the panel `key`. When it was the visible one, the
 * tab after it shows, or the one before it when it was the last.
 */
export function withPanelClosed(state: SidePanelState, key: string): SidePanelState {
  const index = state.panels.findIndex((panel) => panel.key === key);
  if (index === -1) {
    return state;
  }
  const panels = state.panels.filter((panel) => panel.key !== key);
  if (state.shown !== key) {
    return { panels, shown: state.shown };
  }
  const after = panels.at(index);
  const before = index > 0 ? panels.at(index - 1) : undefined;
  return { panels, shown: (after ?? before)?.key ?? null };
}
