<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import * as AlertDialog from '@mannie/plugin-sdk/components/alert-dialog';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';

  import { m } from '../generated/messages.js';
  import { mcpActions } from '../state/mcp-actions.svelte.js';
  import { mcpFailureMessage } from '../utils/mcp-messages.js';

  // Confirms the removal of a server (plugins/mcp/docs/README.md, settings):
  // Mannie stops it, deletes its secrets and tokens, then forgets it and its
  // tools. The permissions for its tools stay listed in the Tools settings
  // until the owner revokes them, and cover nothing without the server.
  const request = $derived(mcpActions.removing);
</script>

<AlertDialog.Root bind:open={mcpActions.removeOpen}>
  <!-- The width needs the variant of the default size, which the component's own maximum uses. -->
  <AlertDialog.Content class="data-[size=default]:sm:max-w-md">
    {#if request !== null}
      <AlertDialog.Header>
        <AlertDialog.Title>{m.mcp_remove_title({ server: request.server.name })}</AlertDialog.Title>
        <AlertDialog.Description>{m.mcp_remove_description()}</AlertDialog.Description>
      </AlertDialog.Header>
      <div class="text-sm" role="status">
        {#if request.removing}
          <span>{m.mcp_removing()}</span>
        {/if}
      </div>
      {#if request.failure !== null}
        <Alert.Root variant="destructive">
          <CircleAlertIcon />
          <Alert.Title>{m.mcp_remove_failed_title()}</Alert.Title>
          <Alert.Description>{mcpFailureMessage(request.failure)}</Alert.Description>
        </Alert.Root>
      {/if}
      <AlertDialog.Footer>
        <AlertDialog.Cancel>{m.mcp_cancel()}</AlertDialog.Cancel>
        <Button
          data-mcp-remove-confirm=""
          disabled={request.removing}
          variant="destructive"
          onclick={() => void mcpActions.confirmRemove(request)}
        >
          {m.mcp_remove_confirm()}
        </Button>
      </AlertDialog.Footer>
    {/if}
  </AlertDialog.Content>
</AlertDialog.Root>
