// The owner's instructions (docs/features/chat/conversations.md, the owner's
// instructions): their limit, and how the Instructions settings count them.

/** The core's limit in characters (`MAX_INSTRUCTIONS_CHARACTERS` in mannie-types), which it checks again. */
export const MAX_INSTRUCTIONS_CHARACTERS = 32_768;

/**
 * The characters of a text as the core counts them: Unicode code points, so
 * an emoji counts once where `String.length` counts its two UTF-16 units.
 */
export function countCharacters(text: string): number {
  return Array.from(text).length;
}
