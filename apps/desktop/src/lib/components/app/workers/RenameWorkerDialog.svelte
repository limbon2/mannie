<script lang="ts">
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Dialog from '#lib/components/ui/dialog/index.js';
  import * as Field from '#lib/components/ui/field/index.js';
  import { Input } from '#lib/components/ui/input/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { workersActions } from '#lib/state/workers-actions.svelte.js';
  import { workersFailureMessage } from '#lib/utils/workers-messages.js';

  // Names a worker (docs/features/workers/worker-activity.md, naming a
  // worker): Mannie and the conversation agent use the owner's name in place
  // of the one in the worker's configuration, and an empty name goes back to
  // that one. The configured name is content, which the worker gave. The core
  // checks the name; a refused one keeps the dialog open with the reason.
  const request = $derived(workersActions.renaming);

  function save(event: SubmitEvent): void {
    event.preventDefault();
    if (request === null) {
      return;
    }
    if (!request.saving) {
      void workersActions.confirmRename(request);
    }
  }
</script>

<Dialog.Root bind:open={workersActions.renameOpen}>
  <Dialog.Content showCloseButton={false}>
    {#if request !== null}
      <form class="flex flex-col gap-4" data-rename-worker-dialog="" onsubmit={save}>
        <Dialog.Header>
          <Dialog.Title>{m.workers_rename_title()}</Dialog.Title>
          <Dialog.Description>{m.workers_rename_description()}</Dialog.Description>
        </Dialog.Header>
        <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 text-sm">
          <dt class="text-muted-foreground">{m.workers_rename_configured()}</dt>
          <dd class="wrap-anywhere" data-content dir="auto">{request.worker.configuredName}</dd>
        </dl>
        <Field.Field data-invalid={request.failure !== null ? true : undefined}>
          <Field.Label for="rename-worker-name">{m.workers_rename_label()}</Field.Label>
          <Input
            id="rename-worker-name"
            aria-invalid={request.failure !== null}
            autocomplete="off"
            spellcheck="false"
            bind:value={request.name}
          />
          {#if request.failure !== null}
            <Field.Error>{workersFailureMessage(request.failure)}</Field.Error>
          {/if}
        </Field.Field>
        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={() => (workersActions.renameOpen = false)}>
            {m.workers_cancel()}
          </Button>
          <Button data-rename-worker-save="" disabled={request.saving} type="submit">{m.workers_save()}</Button>
        </Dialog.Footer>
      </form>
    {/if}
  </Dialog.Content>
</Dialog.Root>
