// The activity bar's selection (ADR-0045, decision 4): the entry of the shown
// view before the owner selects one, the address that the main area last had
// under each entry, views that plugins open, and the webview's storage.
import { describe, expect, it, vi } from 'vitest';

import { Activities, ACTIVITY_STORAGE_KEY } from '../../../../../../apps/desktop/src/lib/state/activities.svelte.js';
import type { NavigationEntry } from '../../../../../../apps/desktop/src/lib/types/shell-slots.js';

vi.mock('$app/navigation', () => ({ goto: vi.fn() }));
vi.mock('$app/paths', () => ({ resolve: (path: string) => path }));

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

function entry(id: string, order: number, view = id): NavigationEntry {
  return { icon: () => ({}), id, label: () => id, order, view };
}

const chat = entry('chat', 0);
const notebooks = entry('notebooks', 20);

/** The selection over the entries, with a storage and the addresses that it navigated to. */
function setUp(storage: Storage | null = new MemoryStorage(), entries = [notebooks, chat]) {
  const navigated: string[] = [];
  const activities = new Activities(storage, (search) => {
    navigated.push(search);
    return Promise.resolve();
  });
  activities.attach({ entries });
  return { activities, navigated };
}

describe('the activity bar', () => {
  it('lists the entries in their order and gives the first the panels that name no entry', () => {
    const { activities } = setUp();

    expect(activities.entries.map((candidate) => candidate.id)).toEqual(['chat', 'notebooks']);
    expect(activities.owns(chat, undefined)).toBe(true);
    expect(activities.owns(notebooks, undefined)).toBe(false);
    expect(activities.owns(notebooks, 'notebooks')).toBe(true);
    expect(activities.owns(chat, 'notebooks')).toBe(false);
  });

  it('selects the entry of the shown view until the owner selects one, and the first without a view', () => {
    const { activities } = setUp();

    expect(activities.current('notebooks')?.id).toBe('notebooks');
    expect(activities.current('chat')?.id).toBe('chat');
    expect(activities.current(null)?.id).toBe('chat');
    expect(activities.current('missing')?.id).toBe('chat');
  });

  it('keeps the selected entry while the main area shows another entry’s view', async () => {
    const { activities } = setUp();

    await activities.select(notebooks);

    expect(activities.current('chat')?.id).toBe('notebooks');
  });

  it('shows an entry’s view the first time, and the address it last had later', async () => {
    const { activities, navigated } = setUp();

    await activities.select(notebooks);
    activities.remember(notebooks, '?view=chat&conversation=c1');
    await activities.select(chat);
    activities.remember(chat, '?view=chat&conversation=c2');
    await activities.select(notebooks);

    expect(navigated).toEqual(['?view=notebooks', '?view=chat', '?view=chat&conversation=c1']);
    expect(activities.addressOf(chat)).toBe('?view=chat&conversation=c2');
  });

  it('opens a view with its parameters, selects its entry, and never lets a parameter name another view', async () => {
    const { activities, navigated } = setUp();

    await activities.openView('notebooks', { notebook: 'n1', view: 'chat' });
    await activities.openView('other');

    expect(navigated).toEqual(['?notebook=n1&view=notebooks', '?view=other']);
    expect(activities.selected).toBe('notebooks');
  });

  it('shows no entry while the selected one does not show, and selects the first again when it is gone', async () => {
    const { activities, navigated } = setUp(new MemoryStorage(), [chat, notebooks]);
    await activities.select(notebooks);
    activities.attach({ entries: [chat] });

    const whileGone = activities.current('notebooks');
    await activities.fallBack();

    expect(whileGone).toBeUndefined();
    expect(activities.current('notebooks')?.id).toBe('chat');
    expect(navigated.at(-1)).toBe('?view=chat');
  });

  it('selects the first entry again without leaving the settings, which lead back to where it was', async () => {
    const { activities, navigated } = setUp(new MemoryStorage(), [chat, notebooks]);
    activities.remember(chat, '?conversation=c1&view=chat');
    await activities.select(notebooks);
    const shownBefore = navigated.length;
    activities.attach({ entries: [chat] });

    const backTo = activities.fallBackInPlace();

    expect(backTo).toBe('?conversation=c1&view=chat');
    expect(activities.selected).toBe('chat');
    expect(navigated).toHaveLength(shownBefore);
  });

  it('keeps the selection and the addresses in the webview’s storage', async () => {
    const storage = new MemoryStorage();
    const first = setUp(storage).activities;

    await first.select(notebooks);
    first.remember(notebooks, '?view=notebooks&notebook=n1');
    const second = setUp(storage).activities;

    expect(second.selected).toBe('notebooks');
    expect(second.addressOf(notebooks)).toBe('?view=notebooks&notebook=n1');
    expect(JSON.parse(storage.items.get(ACTIVITY_STORAGE_KEY) ?? 'null')).toEqual({
      addresses: { notebooks: '?view=notebooks&notebook=n1' },
      selected: 'notebooks',
    });
  });

  it('starts from the shown view when the storage is empty, holds something else, or refuses access', async () => {
    const storage = new MemoryStorage();
    storage.items.set(ACTIVITY_STORAGE_KEY, '{"selected": 3}');
    const unreadable = setUp(storage).activities;
    storage.items.set(ACTIVITY_STORAGE_KEY, 'not JSON');
    const notJson = setUp(storage).activities;
    storage.refuses = true;
    const blocked = setUp(storage).activities;
    const without = setUp(null).activities;

    for (const activities of [unreadable, notJson, blocked, without]) {
      expect(activities.selected).toBeNull();
      expect(activities.current('notebooks')?.id).toBe('notebooks');
    }
    // A storage that refuses writes keeps the selection for the window.
    await blocked.select(notebooks);
    expect(blocked.selected).toBe('notebooks');
  });
});
