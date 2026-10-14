// How the shell picks the renderer of a transcript part (ADR-0020): by the
// part's kind, and for a kind that the core does not know, by its name.
import type { PartView } from '#lib/generated/bindings.js';

/** The ID of the part renderer for a part. */
export function partKind(part: PartView): string {
  return part.content.kind === 'unknown' ? part.content.name : part.content.kind;
}
