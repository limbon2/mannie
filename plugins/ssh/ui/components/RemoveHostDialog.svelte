<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import * as AlertDialog from '@mannie/plugin-sdk/components/alert-dialog';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';

  import { m } from '../generated/messages.js';
  import { sshActions } from '../state/ssh-actions.svelte.js';
  import { sshFailureMessage } from '../utils/ssh-messages.js';

  // Confirms the removal of a host (plugins/ssh/docs/README.md, settings):
  // Mannie deletes its password, then forgets it. Its permissions and rules
  // stay listed in the Tools settings until the owner revokes them.
  const request = $derived(sshActions.removing);
</script>

<AlertDialog.Root bind:open={sshActions.removeOpen}>
  <!-- The width needs the variant of the default size, which the component's own maximum uses. -->
  <AlertDialog.Content class="data-[size=default]:sm:max-w-md">
    {#if request !== null}
      <AlertDialog.Header>
        <AlertDialog.Title>{m.ssh_remove_title({ host: request.host.name })}</AlertDialog.Title>
        <AlertDialog.Description>{m.ssh_remove_description()}</AlertDialog.Description>
      </AlertDialog.Header>
      <div class="text-sm" role="status">
        {#if request.removing}
          <span>{m.ssh_removing()}</span>
        {/if}
      </div>
      {#if request.failure !== null}
        <Alert.Root variant="destructive">
          <CircleAlertIcon />
          <Alert.Title>{m.ssh_remove_failed_title()}</Alert.Title>
          <Alert.Description>{sshFailureMessage(request.failure)}</Alert.Description>
        </Alert.Root>
      {/if}
      <AlertDialog.Footer>
        <AlertDialog.Cancel>{m.ssh_cancel()}</AlertDialog.Cancel>
        <Button
          data-ssh-remove-confirm=""
          disabled={request.removing}
          variant="destructive"
          onclick={() => void sshActions.confirmRemove(request)}
        >
          {m.ssh_remove_confirm()}
        </Button>
      </AlertDialog.Footer>
    {/if}
  </AlertDialog.Content>
</AlertDialog.Root>
