// A section of a source's text as the side panel shows it: the passage that
// a citation opened is one marked piece, counted in UTF-16 as the plugin
// gives its place, kept within the text, and kept when it is empty, so that
// the panel still scrolls to it; and the windows of sections that one read
// returns.
import { describe, expect, it } from 'vitest';

import { textSegments, windowStart } from '../../../ui/utils/notebooks-text.js';

describe('the pieces of a section', () => {
  it('mark the passage between its offsets', () => {
    expect(textSegments('abcdef', { end: 4, start: 2 })).toEqual([
      { marked: false, text: 'ab' },
      { marked: true, text: 'cd' },
      { marked: false, text: 'ef' },
    ]);
    expect(textSegments('abcdef', null)).toEqual([{ marked: false, text: 'abcdef' }]);
  });

  it('count characters in UTF-16, as the plugin gives the offsets', () => {
    expect(textSegments('a👋b', { end: 3, start: 1 })).toEqual([
      { marked: false, text: 'a' },
      { marked: true, text: '👋' },
      { marked: false, text: 'b' },
    ]);
  });

  it('keep the passage within the text, and keep an empty passage', () => {
    expect(textSegments('abc', { end: 99, start: 1 })).toEqual([
      { marked: false, text: 'a' },
      { marked: true, text: 'bc' },
    ]);
    expect(textSegments('abc', { end: 1, start: 2 })).toEqual([
      { marked: false, text: 'ab' },
      { marked: true, text: '' },
      { marked: false, text: 'c' },
    ]);
  });
});

describe('the windows of sections', () => {
  it('start at a multiple of the sections that one read returns', () => {
    expect([0, 63, 64, 130].map(windowStart)).toEqual([0, 0, 64, 128]);
  });
});
