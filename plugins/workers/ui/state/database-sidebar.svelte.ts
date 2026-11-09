// Which databases of the view's sidebar show their tables (plugins/workers/
// docs/databases.md, the view). A database that the owner did not expand or
// collapse shows its tables while its page shows. It is a convenience of the
// window's life.
import { SvelteMap } from 'svelte/reactivity';

/** The expanded databases of the sidebar. */
export class DatabaseSidebar {
  readonly #open = new SvelteMap<string, boolean>();

  /** Whether a database shows its tables; `shown` says whether its page shows. */
  isOpen(key: string, shown: boolean): boolean {
    return this.#open.get(key) ?? shown;
  }

  /** Expands or collapses a database. */
  setOpen(key: string, open: boolean): void {
    this.#open.set(key, open);
  }
}

/** The window's expanded databases. */
export const databaseSidebar = new DatabaseSidebar();
