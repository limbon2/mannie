// What the question card sends from what the owner chose, and how the answers
// read afterwards (docs/features/questions.md).
import { describe, expect, it } from 'vitest';

import type { Question } from '../../../../../../apps/desktop/src/lib/generated/bindings.js';
import {
  answerOf,
  answersOf,
  chosenLabels,
  EMPTY_DRAFT,
} from '../../../../../../apps/desktop/src/lib/utils/question-answers.js';

const QUESTION: Question = {
  multiple: true,
  options: [
    { description: null, label: 'Caching' },
    { description: 'It counts requests.', label: 'Metrics' },
  ],
  text: 'Which extras?',
};

describe('answerOf', () => {
  it('gives no answer while nothing is chosen', () => {
    expect(answerOf(EMPTY_DRAFT)).toBeNull();
  });

  it('gives the chosen options', () => {
    expect(answerOf({ ...EMPTY_DRAFT, selected: [0, 1] })).toEqual({ ownWords: null, selected: [0, 1] });
  });

  it('waits for the owner to write when they chose their own words', () => {
    expect(answerOf({ ...EMPTY_DRAFT, words: '  \n ', writes: true })).toBeNull();
    expect(answerOf({ selected: [1], words: '', writes: true })).toBeNull();
  });

  it('gives the owner’s words without the white space around them', () => {
    expect(answerOf({ ...EMPTY_DRAFT, words: '  on the build machine\n', writes: true })).toEqual({
      ownWords: 'on the build machine',
      selected: [],
    });
  });

  it('leaves out words that the owner wrote and then turned off', () => {
    expect(answerOf({ selected: [0], words: 'not this', writes: false })).toEqual({ ownWords: null, selected: [0] });
  });
});

describe('answersOf', () => {
  it('gives an answer for every question, or none while one is missing', () => {
    const answered = { ...EMPTY_DRAFT, selected: [1] };

    expect(answersOf([answered, answered])).toEqual([
      { ownWords: null, selected: [1] },
      { ownWords: null, selected: [1] },
    ]);
    expect(answersOf([answered, EMPTY_DRAFT])).toBeNull();
  });
});

describe('chosenLabels', () => {
  it('names the options that an answer chose', () => {
    expect(chosenLabels(QUESTION, { ownWords: null, selected: [1, 0] })).toEqual(['Metrics', 'Caching']);
  });

  it('skips a position that the question does not have', () => {
    expect(chosenLabels(QUESTION, { ownWords: 'mine', selected: [7] })).toEqual([]);
  });
});
