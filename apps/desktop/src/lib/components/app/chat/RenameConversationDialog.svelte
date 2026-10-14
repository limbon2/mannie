<script lang="ts">
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Dialog from '#lib/components/ui/dialog/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import { Input } from '#lib/components/ui/input/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { chatActions } from '#lib/state/chat-actions.svelte.js';
  import { chatFailureMessage } from '#lib/utils/chat-errors.js';

  // Renames the conversation that `chatActions.startRename` opened it for. The
  // core checks the title; a refused one keeps the dialog open with the reason.
  const request = $derived(chatActions.renaming);

  function save(event: SubmitEvent): void {
    event.preventDefault();
    if (request !== null) {
      void chatActions.saveRename(request);
    }
  }
</script>

<Dialog.Root bind:open={chatActions.renameOpen}>
  <Dialog.Content showCloseButton={false}>
    {#if request !== null}
      <form class="flex flex-col gap-4" onsubmit={save}>
        <Dialog.Header>
          <Dialog.Title>{m.chat_rename_title()}</Dialog.Title>
          <Dialog.Description>{m.chat_rename_description()}</Dialog.Description>
        </Dialog.Header>
        <Field.Field data-invalid={request.failure !== null ? true : undefined}>
          <Field.Label for="rename-conversation-title">{m.chat_rename_label()}</Field.Label>
          <Input id="rename-conversation-title" aria-invalid={request.failure !== null} bind:value={request.title} />
          {#if request.failure !== null}
            <Field.Error>{chatFailureMessage(request.failure)}</Field.Error>
          {/if}
        </Field.Field>
        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={() => (chatActions.renameOpen = false)}>
            {m.chat_cancel()}
          </Button>
          <Button disabled={request.saving} type="submit">{m.chat_save()}</Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>
