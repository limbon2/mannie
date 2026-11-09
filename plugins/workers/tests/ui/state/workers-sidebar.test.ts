// Whether the "Workers" group of the sidebar is collapsed (plugins/workers/
// docs/worker-activity.md, workers in the sidebar): kept in the webview's
// storage, and expanded when the storage is empty, unreadable or blocked.
import { describe, expect, it } from 'vitest';

import { WORKERS_SIDEBAR_STORAGE_KEY, WorkersSidebar } from '../../../ui/state/workers-sidebar.svelte.js';

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

describe('WorkersSidebar', () => {
  it('starts expanded and remembers that the owner collapsed it', () => {
    const storage = new MemoryStorage();
    const first = new WorkersSidebar(storage);

    expect(first.collapsed).toBe(false);
    first.setOpen(false);

    expect(new WorkersSidebar(storage).collapsed).toBe(true);
    first.setOpen(true);
    expect(new WorkersSidebar(storage).collapsed).toBe(false);
  });

  it('works without storage, or with a stored value it cannot read', () => {
    const blocked = new MemoryStorage();
    blocked.refuses = true;
    const garbled = new MemoryStorage();
    garbled.items.set(WORKERS_SIDEBAR_STORAGE_KEY, '{not json');

    const withBlocked = new WorkersSidebar(blocked);
    withBlocked.setOpen(false);

    expect(new WorkersSidebar(null).collapsed).toBe(false);
    expect(withBlocked.collapsed).toBe(true);
    expect(new WorkersSidebar(garbled).collapsed).toBe(false);
  });
});
