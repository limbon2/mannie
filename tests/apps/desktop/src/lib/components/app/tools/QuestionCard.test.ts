// The card of a question that waits for the owner, and what shows once they
// answered (docs/features/questions.md), rendered on the server side. The model
// wrote the questions and the options, so they carry `data-content`, and
// everything else comes from the catalog.
import { render } from 'svelte/server';
import { afterEach, describe, expect, it, vi } from 'vitest';

import QuestionAnswers from '../../../../../../../../apps/desktop/src/lib/components/app/tools/QuestionAnswers.svelte';
import QuestionCard from '../../../../../../../../apps/desktop/src/lib/components/app/tools/QuestionCard.svelte';
import type { Question } from '../../../../../../../../apps/desktop/src/lib/generated/bindings.js';
import { questionDrafts } from '../../../../../../../../apps/desktop/src/lib/state/question-drafts.svelte.js';

vi.mock('../../../../../../../../apps/desktop/src/lib/generated/bindings.js', () => ({ commands: {} }));

const CONVERSATION = '0199e0f2-0000-7000-8000-000000000001';
const PART = '0199e0f2-0000-7000-8000-000000000002';

const ONE_CHOICE: Question = {
  multiple: false,
  options: [
    { description: 'It has the JSON queries.', label: 'PostgreSQL' },
    { description: null, label: 'SQLite' },
  ],
  text: 'Which database should it use?',
};

const SEVERAL_CHOICES: Question = {
  multiple: true,
  options: [
    { description: null, label: 'Caching' },
    { description: null, label: 'Metrics' },
  ],
  text: 'Which extras should it have?',
};

function card(questions: readonly Question[]): string {
  return render(QuestionCard, {
    props: { conversationId: CONVERSATION, partId: PART, questions, returnFocus: () => undefined },
  }).body;
}

/** Whether the "Send" button of a rendered card is disabled, by its attribute and not by its classes. */
function sendIsDisabled(body: string): boolean {
  const button = /<button[^>]*type="submit"[^>]*>/u.exec(body)?.[0] ?? '';
  return /\sdisabled(?:=""|\s|>)/u.test(button);
}

afterEach(() => {
  questionDrafts.forget(PART);
});

describe('QuestionCard', () => {
  it('shows a question of one choice with its options and the catalog’s words', () => {
    const body = card([ONE_CHOICE]);

    expect(body).toContain('The agent asks');
    expect(body).toContain('Which database should it use?');
    expect(body).toContain('PostgreSQL');
    expect(body).toContain('It has the JSON queries.');
    expect(body).toContain('SQLite');
    expect(body).toContain('Something else');
    expect(body).toContain('Send answer');
    expect(body).toContain("Don't answer");
    expect(body).toContain('role="radiogroup"');
    expect(body).not.toContain('role="checkbox"');
  });

  it('marks what the model wrote as content, and shows it as text', () => {
    const body = card([
      {
        ...ONE_CHOICE,
        text: 'Is <b>this</b> bold?',
      },
    ]);

    expect(body).toContain('data-content');
    expect(body).toContain('dir="auto"');
    expect(body).toContain('Is &lt;b>this&lt;/b> bold?');
    expect(body).not.toContain('<b>this</b>');
  });

  it('offers checkboxes for a question of several choices, and names the plural in the button', () => {
    const body = card([ONE_CHOICE, SEVERAL_CHOICES]);

    expect(body).toContain('role="checkbox"');
    expect(body).toContain('role="radiogroup"');
    expect(body).toContain('Send answers');
  });

  it('keeps Send disabled until every question has an answer', () => {
    expect(sendIsDisabled(card([ONE_CHOICE, SEVERAL_CHOICES]))).toBe(true);

    questionDrafts.chooseOption(PART, 0, 1);
    expect(sendIsDisabled(card([ONE_CHOICE, SEVERAL_CHOICES]))).toBe(true);

    questionDrafts.setOption(PART, 1, 0, true);
    expect(sendIsDisabled(card([ONE_CHOICE, SEVERAL_CHOICES]))).toBe(false);
  });

  it('shows the box for the owner’s words with its note only after they chose it', () => {
    expect(card([ONE_CHOICE])).not.toContain('<textarea');

    questionDrafts.chooseOwnWords(PART, 0);
    const body = card([ONE_CHOICE]);

    expect(body).toContain('<textarea');
    expect(body).toContain('The model reads this. Do not write passwords or keys here.');
    expect(sendIsDisabled(body)).toBe(true);

    questionDrafts.write(PART, 0, 'On the build machine');
    expect(sendIsDisabled(card([ONE_CHOICE]))).toBe(false);
  });

  it('shows the choices that the owner made before they left the conversation', () => {
    questionDrafts.chooseOption(PART, 0, 1);

    const checked = /<button[^>]*role="radio"[^>]*>/gu;
    const radios = card([ONE_CHOICE]).match(checked) ?? [];

    expect(radios).toHaveLength(3);
    expect(radios[0]).toContain('aria-checked="false"');
    expect(radios[1]).toContain('aria-checked="true"');
  });
});

describe('QuestionAnswers', () => {
  function shown(answers: Parameters<typeof render<typeof QuestionAnswers>>[1]): string {
    return render(QuestionAnswers, answers).body;
  }

  it('shows each question with the labels that the owner chose', () => {
    const body = shown({
      props: {
        answers: [
          { ownWords: null, selected: [0] },
          { ownWords: null, selected: [0, 1] },
        ],
        questions: [ONE_CHOICE, SEVERAL_CHOICES],
      },
    });

    expect(body).toContain('aria-label="Your answers"');
    expect(body).toContain('Which database should it use?');
    expect(body).toContain('PostgreSQL');
    expect(body).toContain('Caching and Metrics');
    expect(body).not.toContain('SQLite');
  });

  it('shows the owner’s own words, also beside their choices', () => {
    const body = shown({
      props: {
        answers: [{ ownWords: 'and tracing', selected: [1] }],
        questions: [SEVERAL_CHOICES],
      },
    });

    expect(body).toContain('Metrics');
    expect(body).toContain('and tracing');
  });
});
