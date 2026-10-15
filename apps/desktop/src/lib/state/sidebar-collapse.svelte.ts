// Which parts of the sidebar are collapsed (docs/features/chat/projects.md,
// the sidebar): the "Projects" group and each project. It is a convenience of
// one window, kept in the webview's storage as the theme is. When the storage
// is empty, unreadable or blocked, everything starts expanded, and the
// sidebar works the same without it.

/** The key of the stored state. */
export const SIDEBAR_STORAGE_KEY = 'mannie.chat.sidebar';

/** What the storage holds. */
interface Stored {
  /** The IDs of the collapsed projects. */
  readonly collapsedProjects: readonly string[];
  /** Whether the "Projects" group is collapsed. */
  readonly projectsCollapsed: boolean;
}

function isStored(value: unknown): value is Stored {
  const isObject = typeof value === 'object' && value !== null;
  if (!isObject) {
    return false;
  }
  const { collapsedProjects, projectsCollapsed } = value as Record<string, unknown>;
  const hasProjects = Array.isArray(collapsedProjects) && collapsedProjects.every((id) => typeof id === 'string');
  const hasGroup = typeof projectsCollapsed === 'boolean';
  return hasProjects && hasGroup;
}

/** The webview's storage, or `null` where reading it throws, as in a blocked context. */
export function browserStorage(): Storage | null {
  try {
    return globalThis.localStorage;
  } catch {
    return null;
  }
}

/** The sidebar's collapsed parts. */
export class SidebarCollapse {
  /** The IDs of the collapsed projects. */
  collapsedProjects: string[] = $state([]);
  /** The projects whose list shows every conversation, for this window's life. */
  expandedLists: string[] = $state([]);
  /** Whether the "Projects" group is collapsed. */
  projectsCollapsed = $state(false);
  readonly #storage: Storage | null;

  constructor(storage: Storage | null) {
    this.#storage = storage;
    const stored = this.#read();
    if (stored !== null) {
      this.collapsedProjects = [...stored.collapsedProjects];
      this.projectsCollapsed = stored.projectsCollapsed;
    }
  }

  /** Forgets a project that was removed. */
  forget(id: string): void {
    const kept = this.collapsedProjects.filter((collapsed) => collapsed !== id);
    if (kept.length !== this.collapsedProjects.length) {
      this.collapsedProjects = kept;
      this.#save();
    }
    this.expandedLists = this.expandedLists.filter((expanded) => expanded !== id);
  }

  /** Whether a project shows its conversations. */
  isProjectOpen(id: string): boolean {
    return !this.collapsedProjects.includes(id);
  }

  /** Whether a project's list shows every conversation. */
  listsAll(id: string): boolean {
    return this.expandedLists.includes(id);
  }

  /** Expands the "Projects" group and a project, so that its row and conversations are in view. */
  reveal(id: string): void {
    const changes = this.projectsCollapsed || !this.isProjectOpen(id);
    this.projectsCollapsed = false;
    this.collapsedProjects = this.collapsedProjects.filter((collapsed) => collapsed !== id);
    if (changes) {
      this.#save();
    }
  }

  /** Shows every conversation of a project, or the first few again. */
  setListsAll(id: string, all: boolean): void {
    const others = this.expandedLists.filter((expanded) => expanded !== id);
    this.expandedLists = all ? [...others, id] : others;
  }

  /** Expands or collapses a project. */
  setProjectOpen(id: string, open: boolean): void {
    const others = this.collapsedProjects.filter((collapsed) => collapsed !== id);
    this.collapsedProjects = open ? others : [...others, id];
    this.#save();
  }

  /** Expands or collapses the "Projects" group. */
  setProjectsOpen(open: boolean): void {
    this.projectsCollapsed = !open;
    this.#save();
  }

  #read(): Stored | null {
    try {
      const text = this.#storage?.getItem(SIDEBAR_STORAGE_KEY) ?? null;
      if (text === null) {
        return null;
      }
      const value: unknown = JSON.parse(text);
      return isStored(value) ? value : null;
    } catch {
      // Unreadable storage or a stored value that is not JSON: everything
      // starts expanded.
      return null;
    }
  }

  #save(): void {
    const stored: Stored = { collapsedProjects: this.collapsedProjects, projectsCollapsed: this.projectsCollapsed };
    try {
      this.#storage?.setItem(SIDEBAR_STORAGE_KEY, JSON.stringify(stored));
    } catch {
      // A full or blocked storage keeps the state for this window only.
    }
  }
}

/** The window's collapsed parts of the sidebar. */
export const sidebarCollapse = new SidebarCollapse(browserStorage());
