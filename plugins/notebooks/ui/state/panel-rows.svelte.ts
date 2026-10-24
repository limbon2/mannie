// Which notebooks' rows of the sidebar are open (plugins/notebooks/docs/
// README.md, interface): a row opens when its notebook's page opens or a new
// chat of it starts, so that its chats are in reach, and the owner opens and
// closes rows with their toggles. Rows stay as they are while other views
// show, such as a notebook's chat in the chat view.
import { SvelteSet } from 'svelte/reactivity';

/** The open rows of the window's sidebar, by their notebooks' IDs. */
export class PanelRows {
  readonly #open = new SvelteSet<string>();

  /** Whether the notebook's row is open; read in a component, it follows the row. */
  isOpen(notebookId: string): boolean {
    return this.#open.has(notebookId);
  }

  /** Opens or closes the notebook's row. */
  set(notebookId: string, open: boolean): void {
    if (open) {
      this.#open.add(notebookId);
    } else {
      this.#open.delete(notebookId);
    }
  }
}

/** The open rows of the window's sidebar. */
export const panelRows = new PanelRows();
