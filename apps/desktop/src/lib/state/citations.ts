// The excerpts that a reply's cite links name (ADR-0049, decision 4): the
// items that the conversation's context parts hold, by their providers and
// keys. A reply's text provides them, and a link such as
// `[Guide p. 3](cite:notebooks/<key>)` shows the excerpt that it names, or,
// for a passage that the model read with a tool, opens it where it comes
// from.
import { createContext } from 'svelte';

import type { ContextItemView } from '#lib/generated/bindings.js';

/** An excerpt that a reply can cite, with the provider that gave it. */
export interface CitedItem {
  readonly item: ContextItemView;
  /** The context provider's ID, whose kind words the excerpt and opens it. */
  readonly provider: string;
}

/** The excerpts that the text of one reply can cite. */
export interface Citations {
  /** The stored excerpt of this provider and key, or `undefined` when no context part holds one. */
  readonly find: (provider: string, key: string) => CitedItem | undefined;
}

/** Reads, provides and tells apart the citations of a reply through Svelte's context. */
export const [getCitations, setCitations, hasCitations] = createContext<Citations>();
