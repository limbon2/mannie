// What the owner chose for questions that they have not answered yet
// (docs/features/questions.md): it outlives the conversation's pane, so a
// switch to another conversation and back loses nothing.
import { describe, expect, it } from 'vitest';

import { QuestionDrafts } from '../../../../../../apps/desktop/src/lib/state/question-drafts.svelte.js';
import { EMPTY_DRAFT } from '../../../../../../apps/desktop/src/lib/utils/question-answers.js';

const PART = '0199e0f2-0000-7000-8000-000000000002';
const OTHER_PART = '0199e0f2-0000-7000-8000-000000000003';

describe('QuestionDrafts', () => {
  it('starts every question empty', () => {
    const drafts = new QuestionDrafts();

    expect(drafts.draft(PART, 0)).toEqual(EMPTY_DRAFT);
    expect(drafts.drafts(PART, 2)).toEqual([EMPTY_DRAFT, EMPTY_DRAFT]);
  });

  it('keeps one option in a question of one choice, and swaps it for the owner’s words', () => {
    const drafts = new QuestionDrafts();

    drafts.chooseOption(PART, 0, 1);
    drafts.chooseOption(PART, 0, 2);
    expect(drafts.draft(PART, 0)).toEqual({ selected: [2], words: '', writes: false });

    drafts.chooseOwnWords(PART, 0);
    expect(drafts.draft(PART, 0)).toEqual({ selected: [], words: '', writes: true });

    drafts.chooseOption(PART, 0, 0);
    expect(drafts.draft(PART, 0)).toEqual({ selected: [0], words: '', writes: false });
  });

  it('keeps several options in their order in a question of several choices', () => {
    const drafts = new QuestionDrafts();

    drafts.setOption(PART, 1, 2, true);
    drafts.setOption(PART, 1, 0, true);
    drafts.setOption(PART, 1, 0, true);
    expect(drafts.draft(PART, 1).selected).toEqual([0, 2]);

    drafts.setOption(PART, 1, 2, false);
    expect(drafts.draft(PART, 1).selected).toEqual([0]);
  });

  it('keeps what the owner wrote while their own words are off, so a stray click loses nothing', () => {
    const drafts = new QuestionDrafts();

    drafts.setOwnWords(PART, 0, true);
    drafts.write(PART, 0, 'on the build machine');
    drafts.setOwnWords(PART, 0, false);
    expect(drafts.draft(PART, 0)).toEqual({ selected: [], words: 'on the build machine', writes: false });

    drafts.setOwnWords(PART, 0, true);
    expect(drafts.draft(PART, 0).words).toBe('on the build machine');
  });

  it('keeps each question and each call apart', () => {
    const drafts = new QuestionDrafts();

    drafts.chooseOption(PART, 0, 1);
    drafts.chooseOption(PART, 1, 0);
    drafts.chooseOption(OTHER_PART, 0, 3);

    expect(drafts.draft(PART, 0).selected).toEqual([1]);
    expect(drafts.draft(PART, 1).selected).toEqual([0]);
    expect(drafts.draft(OTHER_PART, 0).selected).toEqual([3]);
  });

  it('forgets a call’s drafts and leaves the others', () => {
    const drafts = new QuestionDrafts();
    drafts.chooseOption(PART, 0, 1);
    drafts.chooseOption(OTHER_PART, 0, 3);

    drafts.forget(PART);
    drafts.forget(PART);

    expect(drafts.draft(PART, 0)).toEqual(EMPTY_DRAFT);
    expect(drafts.draft(OTHER_PART, 0).selected).toEqual([3]);
  });
});
