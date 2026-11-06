<script lang="ts">
  import type { Question, QuestionAnswer } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { labelList } from '#lib/utils/context-kinds.js';
  import { chosenLabels } from '#lib/utils/question-answers.js';

  // What the owner answered to the questions of a call of `ask_owner`
  // (docs/features/questions.md), which stays in view below the call's row.
  // The questions and the labels are model output, and the owner wrote their
  // own words, so all of it is content.
  interface Props {
    answers: readonly QuestionAnswer[];
    questions: readonly Question[];
  }

  let { answers, questions }: Props = $props();

  /** Each question with its answer. The core keeps one answer for each question. */
  const answered = $derived(
    questions.flatMap((question, index) => {
      const answer = answers.at(index);
      return answer === undefined ? [] : [{ answer, labels: chosenLabels(question, answer), question }];
    }),
  );
</script>

<dl
  class="ms-3 flex min-w-0 flex-col gap-2 border-s-2 ps-3 text-sm"
  aria-label={m.tools_question_answers()}
  data-question-answers=""
>
  {#each answered as { answer, labels, question }, index (index)}
    <div class="flex min-w-0 flex-col gap-0.5">
      <dt class="wrap-anywhere text-muted-foreground" data-content dir="auto">{question.text}</dt>
      {#if labels.length > 0}
        <dd class="wrap-anywhere" data-content dir="auto">{labelList(labels)}</dd>
      {/if}
      {#if answer.ownWords !== null}
        <dd class="wrap-anywhere whitespace-pre-wrap" data-content dir="auto">{answer.ownWords}</dd>
      {/if}
    </div>
  {/each}
</dl>
