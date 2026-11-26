<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import * as AlertDialog from '@mannie/plugin-sdk/components/alert-dialog';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import { onDestroy } from 'svelte';

  import { m } from '../generated/messages.js';
  import { proxmoxActions } from '../state/proxmox-actions.svelte.js';
  import { proxmoxFailureMessage } from '../utils/proxmox-messages.js';

  import ProxmoxAllowedVmsDialog from './ProxmoxAllowedVmsDialog.svelte';
  import ProxmoxCertificateDialog from './ProxmoxCertificateDialog.svelte';
  import ProxmoxServerDialog from './ProxmoxServerDialog.svelte';

  const removing = $derived(proxmoxActions.removing);
  const consoleFailure = $derived(proxmoxActions.consoleFailure);

  function confirmRemove(): void {
    if (removing !== null) {
      void proxmoxActions.confirmRemove(removing);
    }
  }

  onDestroy(() => {
    proxmoxActions.closeAll();
  });
</script>

<ProxmoxServerDialog />
<ProxmoxCertificateDialog />
<ProxmoxAllowedVmsDialog />

<AlertDialog.Root bind:open={proxmoxActions.removeOpen}>
  <AlertDialog.Content class="data-[size=default]:sm:max-w-md">
    {#if removing !== null}
      <AlertDialog.Header>
        <AlertDialog.Title>{m.proxmox_remove_title({ server: removing.server.name })}</AlertDialog.Title>
        <AlertDialog.Description>{m.proxmox_remove_description()}</AlertDialog.Description>
      </AlertDialog.Header>

      <div class="text-sm" role="status">
        {#if removing.removing}
          <span>{m.proxmox_removing()}</span>
        {/if}
      </div>

      {#if removing.failure !== null}
        <Alert.Root variant="destructive">
          <CircleAlertIcon />
          <Alert.Title>{m.proxmox_remove_failed_title()}</Alert.Title>
          <Alert.Description>{proxmoxFailureMessage(removing.failure)}</Alert.Description>
        </Alert.Root>
      {/if}

      <AlertDialog.Footer>
        <AlertDialog.Cancel>{m.proxmox_cancel()}</AlertDialog.Cancel>
        <Button
          data-proxmox-remove-confirm=""
          disabled={removing.removing}
          variant="destructive"
          onclick={confirmRemove}
        >
          {m.proxmox_remove_confirm()}
        </Button>
      </AlertDialog.Footer>
    {/if}
  </AlertDialog.Content>
</AlertDialog.Root>

<AlertDialog.Root bind:open={proxmoxActions.consoleFailureOpen}>
  <AlertDialog.Content>
    {#if consoleFailure !== null}
      <AlertDialog.Header>
        <AlertDialog.Title>{m.proxmox_control_error_title()}</AlertDialog.Title>
        <AlertDialog.Description>{proxmoxFailureMessage(consoleFailure)}</AlertDialog.Description>
      </AlertDialog.Header>

      <AlertDialog.Footer>
        <AlertDialog.Cancel>{m.proxmox_close()}</AlertDialog.Cancel>
      </AlertDialog.Footer>
    {/if}
  </AlertDialog.Content>
</AlertDialog.Root>
