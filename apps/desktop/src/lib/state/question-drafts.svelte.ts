// What the owner chose for questions that they have not answered yet
// (docs/features/questions.md). A conversation's pane is destroyed when the
// owner switches to another conversation, so the drafts live here, by the
// part of the call. They stay in memory: reloading the window loses them, and
// the question shows again without them.
import type { PartId } from '#lib/generated/bindings.js';
import { EMPTY_DRAFT, type QuestionDraft } from '#lib/utils/question-answers.js';

/** The window's drafts of answers. */
export class QuestionDrafts {
  /** The drafts of each call that the owner touched, by question. */
  #drafts: Partial<Record<PartId, Partial<Record<number, QuestionDraft>>>> = $state({});

  /** What the owner chose for a question so far. */
  draft(partId: PartId, question: number): QuestionDraft {
    return this.#drafts[partId]?.[question] ?? EMPTY_DRAFT;
  }

  /** The drafts of a call's questions, one for each. */
  drafts(partId: PartId, questions: number): QuestionDraft[] {
    return Array.from({ length: questions }, (_unused, question) => this.draft(partId, question));
  }

  /** In a question of one choice: chooses an option instead of the owner's own words. */
  chooseOption(partId: PartId, question: number, option: number): void {
    this.#change(partId, question, { selected: [option], writes: false });
  }

  /** In a question of one choice: chooses the owner's own words instead of an option. */
  chooseOwnWords(partId: PartId, question: number): void {
    this.#change(partId, question, { selected: [], writes: true });
  }

  /** In a question of several choices: turns an option on or off. */
  setOption(partId: PartId, question: number, option: number, chosen: boolean): void {
    const others = this.draft(partId, question).selected.filter((selected) => selected !== option);
    const selected = chosen ? [...others, option].toSorted((left, right) => left - right) : others;
    this.#change(partId, question, { selected });
  }

  /** In a question of several choices: turns the owner's own words on or off. */
  setOwnWords(partId: PartId, question: number, writes: boolean): void {
    this.#change(partId, question, { writes });
  }

  /** Keeps what the owner wrote. */
  write(partId: PartId, question: number, words: string): void {
    this.#change(partId, question, { words });
  }

  /** Forgets the drafts of a call that no longer waits. */
  forget(partId: PartId): void {
    if (this.#drafts[partId] === undefined) {
      return;
    }
    const { [partId]: _forgotten, ...others } = this.#drafts;
    this.#drafts = others;
  }

  #change(partId: PartId, question: number, change: Partial<QuestionDraft>): void {
    const draft = { ...this.draft(partId, question), ...change };
    this.#drafts[partId] = { ...this.#drafts[partId], [question]: draft };
  }
}

/** The window's drafts of answers. */
export const questionDrafts = new QuestionDrafts();
