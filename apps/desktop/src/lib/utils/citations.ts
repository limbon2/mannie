// The excerpts of a conversation (ADR-0049, decision 4): the items of the
// parts `context` of the owner's messages, which the replies cite by links
// that name their providers and keys, `cite:<provider>/<key>`.
import type { TurnView } from '#lib/generated/bindings.js';
import type { CitedItem } from '#lib/state/citations.js';

/** The scheme of a cite link, as the core writes it into the excerpts. */
export const CITE_SCHEME = 'cite:';

/** What a cite link names. */
export interface CiteTarget {
  /** The item's key, as its provider gave it. */
  readonly key: string;
  /** The context provider's ID. */
  readonly provider: string;
}

/** The map key of an item of a provider. */
function citeKey(provider: string, key: string): string {
  return `${provider}/${key}`;
}

/**
 * What a link's URL cites, when it is a cite link: `cite:` with the
 * provider's ID, a slash and the item's key, both not empty. Any other URL
 * cites nothing.
 */
export function citeTarget(url: string): CiteTarget | null {
  if (!url.startsWith(CITE_SCHEME)) {
    return null;
  }
  const rest = url.slice(CITE_SCHEME.length);
  const slash = rest.indexOf('/');
  const hasProvider = slash > 0;
  const hasKey = slash < rest.length - 1;
  if (!hasProvider || !hasKey) {
    return null;
  }
  return { key: rest.slice(slash + 1), provider: rest.slice(0, slash) };
}

/** The excerpts of every turn, by their providers and keys; a later turn's excerpt of a key replaces an earlier one. */
export function citedItems(turns: readonly TurnView[]): ReadonlyMap<string, CitedItem> {
  const cited = new Map<string, CitedItem>();
  for (const turn of turns) {
    for (const part of turn.parts) {
      if (part.content.kind !== 'context') {
        continue;
      }
      const { provider } = part.content.context;
      for (const item of part.content.context.items) {
        cited.set(citeKey(provider, item.key), { item, provider });
      }
    }
  }
  return cited;
}

/** The stored excerpt of a provider's key, if the conversation holds one. */
export function findCited(cited: ReadonlyMap<string, CitedItem>, provider: string, key: string): CitedItem | undefined {
  return cited.get(citeKey(provider, key));
}
