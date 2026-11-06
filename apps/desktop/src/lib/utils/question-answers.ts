// The owner's answers to the questions of a call of `ask_owner`
// (docs/features/questions.md): what the card sends from what the owner chose,
// and the labels of the options that an answer names.
import type { Question, QuestionAnswer } from '#lib/generated/bindings.js';

/** What the owner chose for one question so far. */
export interface QuestionDraft {
  /** The positions of the options chosen, in the order of the options. */
  readonly selected: readonly number[];
  /** What the owner wrote, which stays while "Something else" is off, so a stray click loses nothing. */
  readonly words: string;
  /** Whether the owner chose to answer in their own words. */
  readonly writes: boolean;
}

/** A question that the owner has not touched. */
export const EMPTY_DRAFT: QuestionDraft = { selected: [], words: '', writes: false };

/**
 * The answer that a draft gives, or `null` while it gives none: nothing is
 * chosen, or "Something else" is chosen and nothing is written yet. The core
 * checks the answer again (ADR-0058, decision 4).
 */
export function answerOf(draft: QuestionDraft): QuestionAnswer | null {
  const words = draft.words.trim();
  const wroteNothing = draft.writes && words === '';
  const choseNothing = !draft.writes && draft.selected.length === 0;
  if (wroteNothing || choseNothing) {
    return null;
  }
  return {
    ownWords: draft.writes ? words : null,
    selected: [...draft.selected],
  };
}

/** The answers that drafts give, one for each question, or `null` while one is missing. */
export function answersOf(drafts: readonly QuestionDraft[]): QuestionAnswer[] | null {
  const answers: QuestionAnswer[] = [];
  for (const draft of drafts) {
    const answer = answerOf(draft);
    if (answer === null) {
      return null;
    }
    answers.push(answer);
  }
  return answers;
}

/** The labels of the options that an answer chose, in their order. */
export function chosenLabels(question: Question, answer: QuestionAnswer): string[] {
  return answer.selected.flatMap((position) => {
    const option = question.options.at(position);
    return option === undefined ? [] : [option.label];
  });
}
