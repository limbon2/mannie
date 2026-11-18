<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Field from '@mannie/plugin-sdk/components/field';
  import { Input } from '@mannie/plugin-sdk/components/input';

  import { m } from '../generated/messages.js';
  import { MAX_NAME_CHARACTERS } from '../generated/plugin.js';
  import { projectActions } from '../state/project-actions.svelte.js';
  import { designerFailureMessage } from '../utils/designer-messages.js';

  const request = $derived(projectActions.name);
  const isBlank = $derived(request === null || request.name.trim() === '');

  function save(event: SubmitEvent): void {
    event.preventDefault();
    const canSave = request !== null && !request.saving && !isBlank;
    if (canSave) {
      void projectActions.confirmName(request);
    }
  }
</script>

<Dialog.Root bind:open={projectActions.nameOpen}>
  <Dialog.Content showCloseButton={false}>
    {#if request !== null}
      <form class="flex flex-col gap-4" data-designer-name-dialog="" onsubmit={save}>
        <Dialog.Header>
          <Dialog.Title>{m.designer_create_title()}</Dialog.Title>
          <Dialog.Description>{m.designer_create_description()}</Dialog.Description>
        </Dialog.Header>

        <Field.Field data-invalid={request.failure !== null ? true : undefined}>
          <Field.Label for="designer-name">{m.designer_name_label()}</Field.Label>
          <Input
            id="designer-name"
            aria-invalid={request.failure !== null}
            autocomplete="off"
            maxlength={MAX_NAME_CHARACTERS}
            bind:value={request.name}
          />
          {#if request.failure !== null}
            <Field.Error>{designerFailureMessage(request.failure)}</Field.Error>
          {/if}
        </Field.Field>

        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={() => (projectActions.nameOpen = false)}>
            {m.designer_cancel()}
          </Button>
          <Button data-designer-name-confirm="" disabled={request.saving || isBlank} type="submit">
            {m.designer_create_confirm()}
          </Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>
