<script lang="ts">
  import CircleAlertIcon from '@lucide/svelte/icons/circle-alert';

  import * as Alert from '#lib/components/ui/alert/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Dialog from '#lib/components/ui/dialog/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { exportActions } from '#lib/state/export-actions.svelte.js';
  import { exportFailureMessage } from '#lib/utils/export-errors.js';
  const canRetry = $derived(exportActions.failure !== null && exportActions.conversationId !== null);
</script>

<Dialog.Root bind:open={exportActions.open}>
  <Dialog.Content showCloseButton={false}>
    <Dialog.Header>
      <Dialog.Title>{m.export_result_title()}</Dialog.Title>
      <Dialog.Description>{m.export_description()}</Dialog.Description>
    </Dialog.Header>
    {#if exportActions.failure !== null}
      <Alert.Root variant="destructive">
        <CircleAlertIcon />
        <Alert.Title>{m.export_failed_title()}</Alert.Title>
        <Alert.Description>{exportFailureMessage(exportActions.failure)}</Alert.Description>
      </Alert.Root>
    {:else if exportActions.result !== null}
      <p role="status">{m.export_saved()}</p>
      {#if exportActions.result.warnings.length > 0}
        <Alert.Root>
          <CircleAlertIcon />
          <Alert.Description>{m.export_partial()}</Alert.Description>
        </Alert.Root>
      {/if}
      <dl class="flex min-w-0 flex-col gap-1">
        <dt class="text-sm text-muted-foreground">{m.export_destination()}</dt>
        <dd class="text-sm break-all select-text" data-content data-export-destination>
          {exportActions.result.destination}
        </dd>
      </dl>
    {/if}
    <Dialog.Footer>
      {#if canRetry}
        <Button
          disabled={exportActions.busy}
          onclick={() => {
            const id = exportActions.conversationId;
            if (id !== null) {
              void exportActions.start(id);
            }
          }}>{m.export_retry()}</Button
        >
      {/if}
      <Button variant="outline" onclick={() => (exportActions.open = false)}>{m.export_close()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
