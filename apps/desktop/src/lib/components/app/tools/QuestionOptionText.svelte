<script lang="ts">
  import * as Field from '#lib/components/ui/field/index.js';
  import type { QuestionOption } from '#lib/generated/bindings.js';

  // The label and the description of an option of a question, beside its radio
  // button or checkbox (docs/features/questions.md). The model wrote both, so
  // they are content.
  interface Props {
    /** The ID of the option's radio button or checkbox. */
    id: string;
    option: QuestionOption;
  }

  let { id, option }: Props = $props();
</script>

<Field.Content class="min-w-0">
  <Field.Label class="font-normal" for={id}>
    <span class="wrap-anywhere" data-content dir="auto">{option.label}</span>
  </Field.Label>
  {#if option.description !== null}
    <Field.Description>
      <span class="wrap-anywhere" data-content dir="auto">{option.description}</span>
    </Field.Description>
  {/if}
</Field.Content>
