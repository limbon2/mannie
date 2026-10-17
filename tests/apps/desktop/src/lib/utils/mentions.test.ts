// The tokens of mentions in the message box's text (docs/features/chat/
// mentions.md): the token that the caret is in, the insertion of a chosen
// entry's token, and the tokens that a message holds.
import { describe, expect, it } from 'vitest';

import { insertMention, mentionQueryAt, tokensInText } from '../../../../../../apps/desktop/src/lib/utils/mentions.js';

/** The query at the end of the text, as the caret leaves it after typing. */
function queryAtEnd(text: string): string | null {
  return mentionQueryAt(text, text.length)?.query ?? null;
}

describe('mentionQueryAt', () => {
  it('finds the token that the caret is in', () => {
    expect(mentionQueryAt('Read @src/ma', 12)).toEqual({ end: 12, query: 'src/ma', start: 5 });
    expect(queryAtEnd('@')).toBe('');
    expect(queryAtEnd('(see @docs')).toBe('docs');
    expect(queryAtEnd('line one\n@notes')).toBe('notes');
  });

  it('reaches to the end of the token after the caret', () => {
    expect(mentionQueryAt('Read @src/main.rs now', 9)).toEqual({ end: 17, query: 'src', start: 5 });
  });

  it('takes spaces after an opening quote', () => {
    expect(queryAtEnd('Read @"my notes/pl')).toBe('my notes/pl');
    expect(mentionQueryAt('@"my notes/plan.md" and', 6)).toEqual({ end: 19, query: 'my n', start: 0 });
  });

  it('finds nothing outside a token', () => {
    expect(queryAtEnd('mail me@example.com')).toBeNull();
    expect(queryAtEnd('@src/main.rs and more')).toBeNull();
    expect(queryAtEnd('@"closed" after')).toBeNull();
    expect(queryAtEnd('no mention here')).toBeNull();
    expect(mentionQueryAt('@first\nsecond', 13)).toBeNull();
  });
});

describe('insertMention', () => {
  it('puts the token in place of the query, with a space and the caret after it', () => {
    const text = 'Read @src/ma please';
    const at = mentionQueryAt(text, 12);
    if (at === null) {
      throw new Error('the caret is in a token');
    }

    const inserted = insertMention(text, at, '@src/main.rs', false);

    expect(inserted).toEqual({ caret: 18, text: 'Read @src/main.rs please' });
  });

  it('adds a space at the end of the text', () => {
    const at = mentionQueryAt('@ma', 3);
    if (at === null) {
      throw new Error('the caret is in a token');
    }

    expect(insertMention('@ma', at, '@main.rs', false)).toEqual({ caret: 9, text: '@main.rs ' });
  });

  it('keeps the caret right after a token that expands', () => {
    const at = mentionQueryAt('Read @do', 8);
    if (at === null) {
      throw new Error('the caret is in a token');
    }

    expect(insertMention('Read @do', at, '@docs/', true)).toEqual({ caret: 11, text: 'Read @docs/' });
  });
});

describe('tokensInText', () => {
  it('lists whole tokens in the order of the text', () => {
    const text = 'Compare @b.rs with @a.rs, then @docs/.';

    expect(tokensInText(text, ['@a.rs', '@docs/', '@b.rs', '@missing'])).toEqual(['@b.rs', '@a.rs', '@docs/']);
  });

  it('skips tokens that the owner changed', () => {
    expect(tokensInText('@docs/guide.md', ['@docs/'])).toEqual([]);
    expect(tokensInText('@main.rs.bak', ['@main.rs'])).toEqual([]);
    expect(tokensInText('x@main.rs', ['@main.rs'])).toEqual([]);
    expect(tokensInText('again @main.rs.bak and @main.rs', ['@main.rs'])).toEqual(['@main.rs']);
  });

  it('finds quoted tokens', () => {
    expect(tokensInText('See @"my notes/" now', ['@"my notes/"'])).toEqual(['@"my notes/"']);
  });
});
