<script lang="ts">
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Dialog from '#lib/components/ui/dialog/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import { Input } from '#lib/components/ui/input/index.js';
  import { Spinner } from '#lib/components/ui/spinner/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { projectActions } from '#lib/state/project-actions.svelte.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';

  // Adds a typed folder as a project (docs/features/chat/projects.md, adding
  // a project from a typed path), for systems where the folder dialog cannot
  // open; "+" opens it with the cause when the dialog failed or Mannie refused
  // the chosen folder. The core checks the folder; a refused one keeps the
  // dialog open with the cause.
  const request = $derived(projectActions.addPath);

  // The example follows the system's form of paths.
  const placeholder = navigator.userAgent.includes('Windows')
    ? m.chat_project_path_placeholder_windows()
    : m.chat_project_path_placeholder_unix();

  function save(event: SubmitEvent): void {
    event.preventDefault();
    if (request !== null) {
      void projectActions.saveAddFromPath(request);
    }
  }
</script>

<Dialog.Root bind:open={projectActions.addPathOpen}>
  <Dialog.Content showCloseButton={false}>
    {#if request !== null}
      <form class="flex flex-col gap-4" onsubmit={save}>
        <Dialog.Header>
          <Dialog.Title>{m.chat_project_path_title()}</Dialog.Title>
          <Dialog.Description>{m.chat_project_path_description()}</Dialog.Description>
        </Dialog.Header>
        <Field.Field data-invalid={request.failure !== null ? true : undefined}>
          <Field.Label for="add-project-path">{m.chat_project_path_label()}</Field.Label>
          <Input
            id="add-project-path"
            class="font-mono"
            aria-invalid={request.failure !== null}
            autocomplete="off"
            {placeholder}
            spellcheck="false"
            bind:value={request.folder}
          />
          {#if request.failure !== null}
            <Field.Error>{chatFailureMessage(request.failure)}</Field.Error>
          {/if}
        </Field.Field>
        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={() => (projectActions.addPathOpen = false)}>
            {m.chat_cancel()}
          </Button>
          <Button disabled={request.saving} type="submit">
            {#if request.saving}
              <Spinner aria-label={m.chat_project_path_checking()} data-icon="inline-start" />
            {/if}
            {m.chat_project_path_save()}
          </Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>
