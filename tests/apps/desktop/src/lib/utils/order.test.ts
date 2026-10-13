import { describe, expect, it } from 'vitest';

import { sortedByOrder } from '../../../../../../apps/desktop/src/lib/utils/order.js';

interface Entry {
  readonly id: string;
  readonly order: number;
}

describe('sortedByOrder', () => {
  it('sorts by order, lowest first, without changing the input', () => {
    const entries: Entry[] = [
      { id: 'c', order: 3 },
      { id: 'a', order: 1 },
      { id: 'b', order: 2 },
    ];

    expect(sortedByOrder(entries).map((entry) => entry.id)).toEqual(['a', 'b', 'c']);
    expect(entries.map((entry) => entry.id)).toEqual(['c', 'a', 'b']);
  });
});
