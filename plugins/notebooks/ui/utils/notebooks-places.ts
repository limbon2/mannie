// Where the Notebooks plugin shows things (plugins/notebooks/docs/README.md,
// interface): its view, with the notebook that it shows, the viewer of a
// source and its settings section, which it opens through the SDK's
// navigation, so the addresses stay the shell's.
import { type NavigateOptions, openView, viewParameters } from '@mannie/plugin-sdk/navigation';

import { panelRows } from '../state/panel-rows.svelte.js';
import { sourceViewer } from '../state/source-viewer.svelte.js';

/** The plugin's view, which is also the ID of its entry of the activity bar. */
export const NOTEBOOKS_VIEW = 'notebooks';

/** The parameter of the view that names the notebook it shows. */
export const NOTEBOOK_PARAMETER = 'notebook';

/** The plugin's settings section. */
export const SETTINGS_SECTION = 'notebooks.settings';

/** The notebook whose page the view shows, or `null`; read in a component, it follows the address. */
export function shownNotebookId(): string | null {
  return viewParameters().get(NOTEBOOK_PARAMETER);
}

/** Shows the view without a notebook, which leads to the most recently changed one. */
export function openNotebooks(): Promise<void> {
  return openView(NOTEBOOKS_VIEW);
}

/**
 * Shows a notebook's page, and opens its row of the sidebar; with `replace`,
 * the address replaces the current one in the history.
 */
export function openNotebook(notebookId: string, options?: NavigateOptions): Promise<void> {
  panelRows.set(notebookId, true);
  return openView(NOTEBOOKS_VIEW, { [NOTEBOOK_PARAMETER]: notebookId }, options);
}

/** Shows a source's text from its start, in the viewer; `opener` takes the focus back when it closes. */
export function openSource(sourceId: string, opener: HTMLElement | null = null): void {
  sourceViewer.show(sourceId, null, opener);
}

/**
 * The source of a passage's key, `<source ID>:<passage number>`, as the
 * context of a conversation or a cite link holds it, or `null` for a key of
 * another shape.
 */
export function sourceOfPassage(key: string): string | null {
  const separator = key.lastIndexOf(':');
  const number = key.slice(separator + 1);
  const isPassage = separator > 0 && /^\d+$/u.test(number);
  return isPassage ? key.slice(0, separator) : null;
}

/** Shows a passage in the viewer, scrolled to it and marked; a key of another shape opens nothing. */
export function openPassage(key: string): void {
  const sourceId = sourceOfPassage(key);
  if (sourceId === null) {
    return;
  }
  sourceViewer.show(sourceId, key);
}
