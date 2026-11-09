// Whether the "Workers" group of the sidebar is collapsed (plugins/workers/
// docs/worker-activity.md, workers in the sidebar). It is a convenience of
// one window, kept in the webview's storage as the core keeps the "Projects"
// group's. When the storage is empty, unreadable or blocked, the group starts
// expanded, and it works the same without it.

/** The key of the stored state. */
export const WORKERS_SIDEBAR_STORAGE_KEY = 'mannie.workers.sidebar';

/** The webview's storage, or `null` where reading it throws, as in a blocked context. */
function browserStorage(): Storage | null {
  try {
    return globalThis.localStorage;
  } catch {
    return null;
  }
}

/** The collapsed state of the "Workers" group. */
export class WorkersSidebar {
  /** Whether the group is collapsed. */
  collapsed = $state(false);
  readonly #storage: Storage | null;

  constructor(storage: Storage | null) {
    this.#storage = storage;
    this.collapsed = this.#read();
  }

  /** Expands or collapses the group. */
  setOpen(open: boolean): void {
    this.collapsed = !open;
    try {
      this.#storage?.setItem(WORKERS_SIDEBAR_STORAGE_KEY, JSON.stringify({ collapsed: this.collapsed }));
    } catch {
      // A full or blocked storage keeps the state for this window only.
    }
  }

  #read(): boolean {
    try {
      const text = this.#storage?.getItem(WORKERS_SIDEBAR_STORAGE_KEY) ?? null;
      if (text === null) {
        return false;
      }
      const value: unknown = JSON.parse(text);
      const isObject = typeof value === 'object' && value !== null;
      if (!isObject) {
        return false;
      }
      return (value as Record<string, unknown>).collapsed === true;
    } catch {
      // Unreadable storage or a stored value that is not JSON: the group
      // starts expanded.
      return false;
    }
  }
}

/** The window's state of the "Workers" group. */
export const workersSidebar = new WorkersSidebar(browserStorage());
