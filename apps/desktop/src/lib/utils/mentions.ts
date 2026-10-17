// The tokens of mentions in the message box's text (docs/features/chat/
// mentions.md): the one that the caret is in, which the "@" list filters by,
// the ones that a message holds, and the insertion of a chosen entry's
// token. The core never reads tokens: a message carries a reference for each
// mention (ADR-0032, decision 1).

/** The longest query that the list looks back for, in characters. */
const MAX_QUERY_LENGTH = 1024;

/** Characters after which "@" starts a mention, besides white space. */
const OPENERS = new Set(['(', '[', '{']);

/** Characters that may follow a token, besides white space. */
const CLOSERS = new Set(['.', ',', ';', ':', '!', '?', ')', ']', '}', "'", '"']);

/** The token that the caret is in. */
export interface MentionQuery {
  /** Where the token ends: after the last character of its path, or after its closing quote. */
  readonly end: number;
  /** What the owner typed after "@" and an opening quote, up to the caret. */
  readonly query: string;
  /** Where the token's "@" is. */
  readonly start: number;
}

/** Where a chosen entry's token leaves the text and the caret. */
export interface Insertion {
  readonly caret: number;
  readonly text: string;
}

/** Whether a character is white space; the empty string, past the text's end, is not. */
function isWhiteSpace(character: string): boolean {
  return /\s/u.test(character);
}

/** Whether a mention can start at `index`: at the start, or after white space or an opening bracket. */
function startsToken(text: string, index: number): boolean {
  const before = text.charAt(index - 1);
  const atStart = index === 0;
  const afterOpener = OPENERS.has(before);
  return atStart || afterOpener || isWhiteSpace(before);
}

/**
 * The token that the caret is in, or `null` outside one: "@" at the start of
 * the text or after white space or an opening bracket, then what the owner
 * typed up to the caret, without white space unless it follows `@"`.
 */
export function mentionQueryAt(text: string, caret: number): MentionQuery | null {
  const earliest = Math.max(0, caret - MAX_QUERY_LENGTH - 2);
  for (let index = caret - 1; index >= earliest; index -= 1) {
    const character = text.charAt(index);
    if (character === '\n') {
      return null;
    }
    const isAt = character === '@';
    if (!isAt || !startsToken(text, index)) {
      continue;
    }
    return queryFrom(text, index, caret);
  }
  return null;
}

/** The token whose "@" is at `at`, up to the caret, or `null` when it holds a break. */
function queryFrom(text: string, at: number, caret: number): MentionQuery | null {
  const opensQuote = text.charAt(at + 1) === '"';
  const caretIsPastQuote = caret > at + 1;
  const quoted = opensQuote && caretIsPastQuote;
  const queryStart = quoted ? at + 2 : at + 1;
  const query = text.slice(queryStart, caret);
  const breaks = quoted ? query.includes('"') : /\s/u.test(query);
  if (breaks) {
    return null;
  }
  const end = quoted ? quotedEnd(text, caret) : unquotedEnd(text, caret);
  return { end, query, start: at };
}

/** Where a token without quotes ends: at the next white space, or the text's end. */
function unquotedEnd(text: string, caret: number): number {
  let end = caret;
  while (continuesToken(text.charAt(end))) {
    end += 1;
  }
  return end;
}

/** Whether a character belongs to a token without quotes. */
function continuesToken(character: string): boolean {
  const pastTheEnd = character === '';
  return !pastTheEnd && !isWhiteSpace(character);
}

/** Where a quoted token ends: after its closing quote, or at the caret without one on the line. */
function quotedEnd(text: string, caret: number): number {
  let end = caret;
  while (staysInQuotes(text.charAt(end))) {
    end += 1;
  }
  const closes = text.charAt(end) === '"';
  return closes ? end + 1 : caret;
}

/** Whether a character belongs to a quoted token before its closing quote. */
function staysInQuotes(character: string): boolean {
  const pastTheEnd = character === '';
  const isQuote = character === '"';
  const isBreak = character === '\n';
  return !(pastTheEnd || isQuote || isBreak);
}

/**
 * Puts a token in place of the token that the caret is in. A token that
 * expands, such as a folder's, keeps the caret right after it, so that the
 * owner can go on into the folder; any other token gets a space after it.
 */
export function insertMention(text: string, at: MentionQuery, token: string, expands: boolean): Insertion {
  const before = text.slice(0, at.start);
  const after = text.slice(at.end);
  if (expands) {
    return { caret: before.length + token.length, text: before + token + after };
  }
  const spaced = isWhiteSpace(after.charAt(0)) ? after : ` ${after}`;
  return { caret: before.length + token.length + 1, text: before + token + spaced };
}

/**
 * Whether a token ends at `index`: at the text's end or white space, maybe
 * after closing punctuation, as in "see @a.rs." but not in "@a.rs.bak".
 */
function endsAt(text: string, index: number): boolean {
  let next = index;
  while (CLOSERS.has(text.charAt(next))) {
    next += 1;
  }
  const after = text.charAt(next);
  const endsText = after === '';
  return endsText || isWhiteSpace(after);
}

/** Whether `token` stands at `index` as a whole token. */
function standsAt(text: string, token: string, index: number): boolean {
  const ends = endsAt(text, index + token.length);
  return ends && startsToken(text, index);
}

/** Where `token` first stands in the text as a whole token, or -1. */
function firstStanding(text: string, token: string): number {
  let index = text.indexOf(token);
  while (index !== -1) {
    if (standsAt(text, token, index)) {
      return index;
    }
    index = text.indexOf(token, index + 1);
  }
  return -1;
}

/**
 * The tokens among `tokens` that the text holds as whole tokens, in the order
 * in which they first appear. A token that the owner changed, such as a
 * folder's token followed by more of a path, no longer counts.
 */
export function tokensInText(text: string, tokens: Iterable<string>): string[] {
  const found: { index: number; token: string }[] = [];
  for (const token of tokens) {
    const index = firstStanding(text, token);
    if (index !== -1) {
      found.push({ index, token });
    }
  }
  found.sort((left, right) => left.index - right.index);
  return found.map((entry) => entry.token);
}
