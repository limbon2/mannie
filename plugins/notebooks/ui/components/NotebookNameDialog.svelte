<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';

  import { m } from '../generated/messages.js';
  import { MAX_NAME_CHARACTERS } from '../generated/plugin.js';
  import { notebookActions } from '../state/notebook-actions.svelte.js';
  import { notebooksFailureMessage } from '../utils/notebooks-messages.js';

  // Names a new notebook, or a notebook to rename (plugins/notebooks/docs/
  // README.md, notebooks). The plugin checks the name, which no other
  // notebook may have without regard to case; a refused one keeps the dialog
  // open with the reason. A new notebook's page opens once it exists.
  const request = $derived(notebookActions.name);
  const isBlank = $derived(request === null || request.name.trim() === '');

  function save(event: SubmitEvent): void {
    event.preventDefault();
    const canSave = request !== null && !request.saving && !isBlank;
    if (canSave) {
      void notebookActions.confirmName(request);
    }
  }
</script>

<Dialog.Root bind:open={notebookActions.nameOpen}>
  <Dialog.Content showCloseButton={false}>
    {#if request !== null}
      <form class="flex flex-col gap-4" data-notebooks-name-dialog="" onsubmit={save}>
        <Dialog.Header>
          <Dialog.Title>
            {request.notebook === null ? m.notebooks_create_title() : m.notebooks_rename_title()}
          </Dialog.Title>
          <Dialog.Description>
            {request.notebook === null ? m.notebooks_create_description() : m.notebooks_rename_description()}
          </Dialog.Description>
        </Dialog.Header>
        <Field.Field data-invalid={request.failure !== null ? true : undefined}>
          <Field.Label for="notebooks-name">{m.notebooks_name_label()}</Field.Label>
          <Input
            id="notebooks-name"
            aria-invalid={request.failure !== null}
            autocomplete="off"
            maxlength={MAX_NAME_CHARACTERS}
            bind:value={request.name}
          />
          {#if request.failure !== null}
            <Field.Error>{notebooksFailureMessage(request.failure)}</Field.Error>
          {/if}
        </Field.Field>
        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={() => (notebookActions.nameOpen = false)}>
            {m.notebooks_cancel()}
          </Button>
          <Button data-notebooks-name-confirm="" disabled={request.saving || isBlank} type="submit">
            {request.notebook === null ? m.notebooks_create_confirm() : m.notebooks_rename_confirm()}
          </Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>
