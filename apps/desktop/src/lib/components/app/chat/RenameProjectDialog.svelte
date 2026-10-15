<script lang="ts">
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Dialog from '#lib/components/ui/dialog/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import { Input } from '#lib/components/ui/input/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { projectActions } from '#lib/state/project-actions.svelte.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';

  // Renames the project that `projectActions.startRename` opened it for
  // (docs/features/chat/projects.md, renaming and removing). The core checks
  // the name; a refused one keeps the dialog open with the reason.
  const request = $derived(projectActions.renaming);

  function save(event: SubmitEvent): void {
    event.preventDefault();
    if (request !== null) {
      void projectActions.saveRename(request);
    }
  }
</script>

<Dialog.Root bind:open={projectActions.renameOpen}>
  <Dialog.Content showCloseButton={false}>
    {#if request !== null}
      <form class="flex flex-col gap-4" onsubmit={save}>
        <Dialog.Header>
          <Dialog.Title>{m.chat_project_rename_title()}</Dialog.Title>
          <Dialog.Description>{m.chat_project_rename_description()}</Dialog.Description>
        </Dialog.Header>
        <Field.Field data-invalid={request.failure !== null ? true : undefined}>
          <Field.Label for="rename-project-name">{m.chat_project_rename_label()}</Field.Label>
          <Input id="rename-project-name" aria-invalid={request.failure !== null} bind:value={request.name} />
          {#if request.failure !== null}
            <Field.Error>{chatFailureMessage(request.failure)}</Field.Error>
          {/if}
        </Field.Field>
        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={() => (projectActions.renameOpen = false)}>
            {m.chat_cancel()}
          </Button>
          <Button disabled={request.saving} type="submit">{m.chat_save()}</Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>
