// Ordering of slot entries: a feature gives each entry an `order`, and the
// shell lists the entries of a slot by it.

interface OrderedEntry {
  readonly order: number;
}

/** The entries sorted by their `order`, lowest first. */
export function sortedByOrder<T extends OrderedEntry>(entries: readonly T[]): T[] {
  return [...entries].sort((left, right) => left.order - right.order);
}
