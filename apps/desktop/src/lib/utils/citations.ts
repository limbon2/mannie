// The excerpts of a conversation (ADR-0049, decision 4): the items of the
// parts `context` of the owner's messages and the items that tool calls'
// results cite (ADR-0054, decision 1), which the replies cite by links that
// name their providers and keys, `cite:<provider>/<key>`. A key may end with
// a line fragment, `#L<first>-L<last>`, which names lines of the item
// (ADR-0054, decision 2).
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

/** Lines of an item that a cite link names, numbered from 1. */
export interface CitedLines {
  readonly first: number;
  readonly last: number;
}

/** The parts of a key that a cite link names: the item's key and the lines that its fragment names, if it names any. */
export interface CiteKey {
  /** The item's key, without the fragment. */
  readonly key: string;
  readonly lines: CitedLines | null;
}

/** A line fragment: `#L12` for one line, `#L12-L30` for several, the last not before the first. */
const LINE_FRAGMENT = /#L([1-9]\d{0,8})(?:-L([1-9]\d{0,8}))?$/u;

/**
 * A cite link's key taken apart (ADR-0054, decision 2). A fragment that does
 * not name lines stays part of the key, and so does a range whose last line
 * comes before its first.
 */
export function citeKeyParts(key: string): CiteKey {
  const match = LINE_FRAGMENT.exec(key);
  if (match === null) {
    return { key, lines: null };
  }
  if (match.index === 0) {
    return { key, lines: null };
  }
  const first = Number(match[1]);
  // The range's end is an optional group, which may not have matched.
  const end = match.at(2);
  const last = end === undefined ? first : Number(end);
  if (last < first) {
    return { key, lines: null };
  }
  return { key: key.slice(0, match.index), lines: { first, last } };
}

/** The text of an item around the lines that a citation names, which the popover marks. */
export interface MarkedText {
  readonly after: string;
  readonly before: string;
  readonly marked: string;
}

/**
 * An item's text split around the lines that a citation names, or `null`
 * when the item has no such lines. A range past the item's end marks up to
 * its last line.
 */
export function markedLines(text: string, lines: CitedLines): MarkedText | null {
  const all = text.split('\n');
  if (lines.first > all.length) {
    return null;
  }
  const last = Math.min(lines.last, all.length);
  const before = all.slice(0, lines.first - 1);
  const after = all.slice(last);
  return {
    after: after.length === 0 ? '' : `\n${after.join('\n')}`,
    before: before.length === 0 ? '' : `${before.join('\n')}\n`,
    marked: all.slice(lines.first - 1, last).join('\n'),
  };
}

/**
 * The excerpts of every turn, by their providers and keys: the items of
 * context parts and those that tool calls cite. A later excerpt of a key
 * replaces an earlier one.
 */
export function citedItems(turns: readonly TurnView[]): ReadonlyMap<string, CitedItem> {
  const cited = new Map<string, CitedItem>();
  for (const turn of turns) {
    for (const part of turn.parts) {
      if (part.content.kind === 'context') {
        const { provider } = part.content.context;
        for (const item of part.content.context.items) {
          cited.set(citeKey(provider, item.key), { item, provider });
        }
      } else if (part.content.kind === 'tool_call') {
        for (const item of part.content.call.cited ?? []) {
          cited.set(citeKey(item.provider, item.key), { item, provider: item.provider });
        }
      }
    }
  }
  return cited;
}

/** The stored excerpt of a provider's key, without its line fragment, if the conversation holds one. */
export function findCited(cited: ReadonlyMap<string, CitedItem>, provider: string, key: string): CitedItem | undefined {
  return cited.get(citeKey(provider, citeKeyParts(key).key));
}
