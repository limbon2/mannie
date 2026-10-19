<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import * as AlertDialog from '@mannie/plugin-sdk/components/alert-dialog';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';

  import { m } from '../generated/messages.js';
  import { workersActions } from '../state/workers-actions.svelte.js';
  import { workersFailureMessage } from '../utils/workers-messages.js';

  // Confirms the removal of a worker (plugins/workers/docs/
  // worker-activity.md, removing a worker): the worker forgets this app and
  // cancels its tasks, and an offline worker keeps its pairing until its
  // host resets it. The worker's name is content, which the worker gave.
  const request = $derived(workersActions.removing);
</script>

<AlertDialog.Root bind:open={workersActions.removeOpen}>
  <!-- The width needs the variant of the default size, which the component's own maximum uses. -->
  <AlertDialog.Content class="data-[size=default]:sm:max-w-md">
    {#if request !== null}
      <AlertDialog.Header>
        <AlertDialog.Title>{m.workers_remove_title()}</AlertDialog.Title>
        <p class="text-sm font-medium wrap-anywhere" data-content dir="auto">{request.worker.name}</p>
        <AlertDialog.Description>{m.workers_remove_description()}</AlertDialog.Description>
      </AlertDialog.Header>
      {#if request.failure !== null}
        <Alert.Root variant="destructive">
          <CircleAlertIcon />
          <Alert.Title>{m.workers_remove_failed_title()}</Alert.Title>
          <Alert.Description>{workersFailureMessage(request.failure)}</Alert.Description>
        </Alert.Root>
      {/if}
      <AlertDialog.Footer>
        <AlertDialog.Cancel>{m.workers_cancel()}</AlertDialog.Cancel>
        <Button
          data-remove-worker-confirm=""
          disabled={request.removing}
          variant="destructive"
          onclick={() => void workersActions.confirmRemove(request)}
        >
          {m.workers_remove_confirm()}
        </Button>
      </AlertDialog.Footer>
    {/if}
  </AlertDialog.Content>
</AlertDialog.Root>
