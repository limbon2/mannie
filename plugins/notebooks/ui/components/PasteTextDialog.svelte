<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';
  import { Textarea } from '@mannie/plugin-sdk/components/textarea';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';

  import { m } from '../generated/messages.js';
  import { MAX_TITLE_CHARACTERS } from '../generated/plugin.js';
  import { notebookActions } from '../state/notebook-actions.svelte.js';
  import { notebooksFailureMessage } from '../utils/notebooks-messages.js';

  // "Paste text" (plugins/notebooks/docs/README.md, sources): a title and a
  // text of any length that the owner writes or pastes, which becomes a
  // source of the notebook (plugin ADR-0002). The plugin checks both again; a
  // refused text keeps the dialog open with the reason.
  const request = $derived(notebookActions.paste);
  const isIncomplete = $derived(request === null || request.title.trim() === '' || request.text.trim() === '');

  function save(event: SubmitEvent): void {
    event.preventDefault();
    const canSave = request !== null && !request.saving && !isIncomplete;
    if (canSave) {
      void notebookActions.confirmPaste(request);
    }
  }
</script>

<Dialog.Root bind:open={notebookActions.pasteOpen}>
  <Dialog.Content class="sm:max-w-2xl" showCloseButton={false}>
    {#if request !== null}
      <form class="flex min-w-0 flex-col gap-4" data-notebooks-paste-dialog="" onsubmit={save}>
        <Dialog.Header>
          <Dialog.Title>{m.notebooks_paste_title()}</Dialog.Title>
          <Dialog.Description>{m.notebooks_paste_description({ name: request.notebook.name })}</Dialog.Description>
        </Dialog.Header>
        <Field.Field>
          <Field.Label for="notebooks-paste-title">{m.notebooks_paste_title_label()}</Field.Label>
          <Input
            id="notebooks-paste-title"
            autocomplete="off"
            maxlength={MAX_TITLE_CHARACTERS}
            bind:value={request.title}
          />
        </Field.Field>
        <Field.Field>
          <Field.Label for="notebooks-paste-text">{m.notebooks_paste_text_label()}</Field.Label>
          <Textarea id="notebooks-paste-text" class="max-h-[50vh] min-h-48" bind:value={request.text} />
        </Field.Field>
        <div class="text-sm" role="status">
          {#if request.saving}
            <span>{m.notebooks_pasting()}</span>
          {/if}
        </div>
        {#if request.failure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Title>{m.notebooks_paste_failed_title()}</Alert.Title>
            <Alert.Description>{notebooksFailureMessage(request.failure)}</Alert.Description>
          </Alert.Root>
        {/if}
        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={() => (notebookActions.pasteOpen = false)}>
            {m.notebooks_cancel()}
          </Button>
          <Button data-notebooks-paste-confirm="" disabled={request.saving || isIncomplete} type="submit">
            {m.notebooks_paste_confirm()}
          </Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>
