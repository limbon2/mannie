// The Instructions settings count characters as the core does: Unicode code
// points, never UTF-16 units or UTF-8 bytes (docs/features/chat/
// conversations.md, the owner's instructions).
import { describe, expect, it } from 'vitest';

import {
  countCharacters,
  MAX_INSTRUCTIONS_CHARACTERS,
} from '../../../../../../apps/desktop/src/lib/utils/chat-instructions.js';

describe('instructions', () => {
  it('count each character once, whatever its size in bytes', () => {
    expect(countCharacters('')).toBe(0);
    expect(countCharacters('Answer in English.')).toBe(18);
    expect(countCharacters('Réponds en français.')).toBe(20);
    expect(countCharacters('日本語で答えて')).toBe(7);
    expect(countCharacters('🙂🙂')).toBe(2);
  });

  it('allow as many characters as the core', () => {
    expect(MAX_INSTRUCTIONS_CHARACTERS).toBe(32_768);
    expect(countCharacters('🙂'.repeat(MAX_INSTRUCTIONS_CHARACTERS))).toBe(MAX_INSTRUCTIONS_CHARACTERS);
  });
});
