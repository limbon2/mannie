<script lang="ts">
  import { tick } from 'svelte';

  import { Button } from '#lib/components/ui/button/index.js';
  import { Checkbox } from '#lib/components/ui/checkbox/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import * as RadioGroup from '#lib/components/ui/radio-group/index.js';
  import { Textarea } from '#lib/components/ui/textarea/index.js';
  import type { ConversationId, PartId, Question, QuestionReply } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { questionDrafts } from '#lib/state/question-drafts.svelte.js';
  import { toolsActions } from '#lib/state/tools-actions.svelte.js';
  import { answersOf } from '#lib/utils/question-answers.js';

  import QuestionOptionText from './QuestionOptionText.svelte';

  // A call of `ask_owner` that waits for the owner (docs/features/questions.md).
  // The questions and their options are model output, so they carry
  // `data-content` and show as plain text, while the title, the choice of the
  // owner's own words and the buttons come from the catalog and stand outside
  // every piece of content (T-82). Nothing here takes the focus by itself: an
  // arrow key that moves onto "Something else" must not move the focus away
  // from the options (WCAG 3.2.2), and the box follows them in the tab order.
  interface Props {
    conversationId: ConversationId;
    partId: PartId;
    questions: readonly Question[];
    /** Called when the card goes away after the owner replied from inside it. */
    returnFocus: () => void;
  }

  let { conversationId, partId, questions, returnFocus }: Props = $props();

  /** The value of a question's radio button for the owner's own words. */
  const OWN_WORDS = 'own-words';

  let form = $state<HTMLFormElement>();
  /** Whether the focus was in the card when the owner last replied. */
  let repliedFromInside = false;

  const sending = $derived(toolsActions.deciding.includes(partId));
  const answers = $derived(answersOf(questionDrafts.drafts(partId, questions.length)));

  /** The value of a question's radio group: the chosen option, the owner's own words, or nothing yet. */
  function radioValue(question: number): string {
    const draft = questionDrafts.draft(partId, question);
    if (draft.writes) {
      return OWN_WORDS;
    }
    return String(draft.selected.at(0) ?? '');
  }

  function chooseRadio(question: number, value: string): void {
    if (value === OWN_WORDS) {
      questionDrafts.chooseOwnWords(partId, question);
    } else {
      questionDrafts.chooseOption(partId, question, Number(value));
    }
  }

  function write(question: number, event: Event): void {
    const box = event.currentTarget;
    if (box instanceof HTMLTextAreaElement) {
      questionDrafts.write(partId, question, box.value);
    }
  }

  /**
   * Sends the reply. The controls are disabled while it is on its way, which
   * drops the focus of the one that sent it, so a refusal, which keeps the
   * card, gives the focus back.
   */
  async function reply(answerReply: QuestionReply): Promise<void> {
    const focused = document.activeElement;
    repliedFromInside = form?.contains(focused) ?? false;
    const accepted = await toolsActions.answer(conversationId, partId, answerReply);
    if (accepted || !repliedFromInside) {
      return;
    }
    repliedFromInside = false;
    await tick();
    if (focused instanceof HTMLElement) {
      focused.focus();
    }
  }

  function send(event: SubmitEvent): void {
    event.preventDefault();
    const canSend = answers !== null && !sending;
    if (canSend) {
      void reply({ answers, reply: 'answered' });
    }
  }

  // The teardown runs once the card left the page, which drops a focus that
  // was in it. Without this, a keyboard user would start again from the top of
  // the window.
  $effect(() => {
    return () => {
      if (repliedFromInside) {
        returnFocus();
      }
    };
  });
</script>

<form
  bind:this={form}
  class="flex flex-col gap-4 rounded-md border bg-muted/40 p-3"
  data-question-card=""
  onsubmit={send}
>
  <p class="text-sm font-medium">{m.tools_question_title()}</p>
  {#each questions as question, questionIndex (questionIndex)}
    {const draft = $derived(questionDrafts.draft(partId, questionIndex))}
    {const prefix = $derived(`question-${partId}-${questionIndex}`)}
    <Field.Set class="min-w-0" data-question="">
      <Field.Legend variant="label">
        <span class="wrap-anywhere" data-content dir="auto">{question.text}</span>
      </Field.Legend>
      {#if question.multiple}
        <Field.Group class="gap-3">
          {#each question.options as option, optionIndex (optionIndex)}
            {const id = $derived(`${prefix}-${optionIndex}`)}
            <Field.Field orientation="horizontal">
              <Checkbox
                {id}
                checked={draft.selected.includes(optionIndex)}
                disabled={sending}
                onCheckedChange={(checked: boolean) =>
                  questionDrafts.setOption(partId, questionIndex, optionIndex, checked)}
              />
              <QuestionOptionText {id} {option} />
            </Field.Field>
          {/each}
          <Field.Field orientation="horizontal">
            <Checkbox
              id="{prefix}-own"
              checked={draft.writes}
              disabled={sending}
              onCheckedChange={(checked: boolean) => questionDrafts.setOwnWords(partId, questionIndex, checked)}
            />
            <Field.Label class="font-normal" for="{prefix}-own">{m.tools_question_other()}</Field.Label>
          </Field.Field>
        </Field.Group>
      {:else}
        <RadioGroup.Root
          disabled={sending}
          value={radioValue(questionIndex)}
          onValueChange={(value: string) => chooseRadio(questionIndex, value)}
        >
          {#each question.options as option, optionIndex (optionIndex)}
            {const id = $derived(`${prefix}-${optionIndex}`)}
            <Field.Field orientation="horizontal">
              <RadioGroup.Item {id} value={String(optionIndex)} />
              <QuestionOptionText {id} {option} />
            </Field.Field>
          {/each}
          <Field.Field orientation="horizontal">
            <RadioGroup.Item id="{prefix}-own" value={OWN_WORDS} />
            <Field.Label class="font-normal" for="{prefix}-own">{m.tools_question_other()}</Field.Label>
          </Field.Field>
        </RadioGroup.Root>
      {/if}
      {#if draft.writes}
        <Field.Field>
          <Field.Label for="{prefix}-words">{m.tools_question_own_words()}</Field.Label>
          <Textarea
            id="{prefix}-words"
            aria-describedby="{prefix}-note"
            disabled={sending}
            value={draft.words}
            oninput={(event: Event) => write(questionIndex, event)}
          />
          <Field.Description id="{prefix}-note">{m.tools_question_own_words_note()}</Field.Description>
        </Field.Field>
      {/if}
    </Field.Set>
  {/each}
  <div class="flex flex-wrap gap-2">
    <Button disabled={sending || answers === null} size="sm" type="submit">
      {m.tools_question_send({ count: questions.length })}
    </Button>
    <Button disabled={sending} size="sm" variant="outline" onclick={() => void reply({ reply: 'declined' })}>
      {m.tools_question_decline()}
    </Button>
  </div>
</form>
