// Which parts of the sidebar are collapsed (docs/features/chat/projects.md,
// the sidebar): kept in the webview's storage, and expanded when the storage
// is empty, unreadable or blocked.
import { describe, expect, it } from 'vitest';

import {
  SIDEBAR_STORAGE_KEY,
  SidebarCollapse,
} from '../../../../../../apps/desktop/src/lib/state/sidebar-collapse.svelte.js';

/** A storage in memory, which can refuse every access. */
class MemoryStorage implements Storage {
  readonly items = new Map<string, string>();
  refuses = false;

  get length(): number {
    return this.items.size;
  }

  clear(): void {
    this.items.clear();
  }

  getItem(key: string): string | null {
    this.#check();
    return this.items.get(key) ?? null;
  }

  key(index: number): string | null {
    return [...this.items.keys()][index] ?? null;
  }

  removeItem(key: string): void {
    this.items.delete(key);
  }

  setItem(key: string, value: string): void {
    this.#check();
    this.items.set(key, value);
  }

  #check(): void {
    if (this.refuses) {
      throw new DOMException('The storage is blocked.', 'SecurityError');
    }
  }
}

describe('SidebarCollapse', () => {
  it('starts expanded and remembers what the owner collapsed', () => {
    const storage = new MemoryStorage();
    const first = new SidebarCollapse(storage);

    expect(first.projectsCollapsed).toBe(false);
    expect(first.isProjectOpen('app')).toBe(true);
    first.setProjectOpen('app', false);
    first.setProjectsOpen(false);

    const later = new SidebarCollapse(storage);
    expect(later.isProjectOpen('app')).toBe(false);
    expect(later.projectsCollapsed).toBe(true);
  });

  it('expands a project and the group to show where a conversation works', () => {
    const collapse = new SidebarCollapse(new MemoryStorage());
    collapse.setProjectOpen('app', false);
    collapse.setProjectsOpen(false);

    collapse.reveal('app');

    expect(collapse.isProjectOpen('app')).toBe(true);
    expect(collapse.projectsCollapsed).toBe(false);
  });

  it('forgets a removed project', () => {
    const storage = new MemoryStorage();
    const collapse = new SidebarCollapse(storage);
    collapse.setProjectOpen('gone', false);
    collapse.setListsAll('gone', true);

    collapse.forget('gone');

    expect(collapse.listsAll('gone')).toBe(false);
    expect(new SidebarCollapse(storage).isProjectOpen('gone')).toBe(true);
  });

  it('works without storage, or with a stored value it cannot read', () => {
    const blocked = new MemoryStorage();
    blocked.refuses = true;
    const garbled = new MemoryStorage();
    garbled.items.set(SIDEBAR_STORAGE_KEY, '{not json');

    const withoutStorage = new SidebarCollapse(null);
    const withBlocked = new SidebarCollapse(blocked);
    withBlocked.setProjectOpen('app', false);

    expect(withoutStorage.isProjectOpen('app')).toBe(true);
    expect(withBlocked.isProjectOpen('app')).toBe(false);
    expect(new SidebarCollapse(garbled).projectsCollapsed).toBe(false);
  });

  it('shows every conversation of a project only while the owner asks for it', () => {
    const collapse = new SidebarCollapse(new MemoryStorage());

    collapse.setListsAll('app', true);
    expect(collapse.listsAll('app')).toBe(true);
    collapse.setListsAll('app', false);
    expect(collapse.listsAll('app')).toBe(false);
  });
});
