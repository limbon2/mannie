<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import { Checkbox } from '@mannie/plugin-sdk/components/checkbox';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import * as Table from '@mannie/plugin-sdk/components/table';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';

  import { m } from '../generated/messages.js';
  import { proxmoxActions } from '../state/proxmox-actions.svelte.js';
  import { isCertificateFailure, proxmoxFailureMessage } from '../utils/proxmox-messages.js';

  const request = $derived(proxmoxActions.vms);
  const isBusy = $derived(request !== null && (request.loading || request.saving));
  const hasVms = $derived((request?.vms.length ?? 0) > 0);
  const loadFailure = $derived(hasVms ? null : (request?.failure ?? null));
  const saveFailure = $derived(hasVms ? (request?.failure ?? null) : null);
  const certError = $derived(loadFailure !== null && isCertificateFailure(loadFailure));

  function closeDialog(): void {
    proxmoxActions.vmsOpen = false;
  }

  function retryLoad(): void {
    if (request !== null) {
      void proxmoxActions.loadDiscoverableVms(request);
    }
  }

  function inspectCertificateFromVms(): void {
    if (request !== null) {
      proxmoxActions.vmsOpen = false;
      proxmoxActions.startInspectCert(request.server, { continueToVms: true });
    }
  }

  function selectAll(): void {
    request?.selectAll();
  }

  function clearAll(): void {
    request?.clearAll();
  }

  function toggleVm(vmid: number, checked: boolean): void {
    request?.toggleVm(vmid, checked);
  }

  function saveAllowlist(): void {
    if (request === null) {
      return;
    }

    if (request.saving) {
      return;
    }

    void proxmoxActions.confirmAllowedVms(request);
  }
</script>

<Dialog.Root bind:open={proxmoxActions.vmsOpen}>
  <Dialog.Content class="sm:max-w-2xl" showCloseButton={false}>
    {#if request !== null}
      <div class="flex flex-col gap-4" data-proxmox-vms-dialog="">
        <Dialog.Header>
          <Dialog.Title>{m.proxmox_vms_title({ server: request.server.name })}</Dialog.Title>
          <Dialog.Description>{m.proxmox_vms_description()}</Dialog.Description>
        </Dialog.Header>

        {#if request.loading}
          <div class="flex items-center gap-2 py-6 text-sm text-muted-foreground" role="status">
            <Spinner aria-hidden="true" />
            <span>{m.proxmox_vms_loading({ server: request.server.name })}</span>
          </div>
        {:else if loadFailure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Title>{m.proxmox_vms_failed_title({ server: request.server.name })}</Alert.Title>
            <Alert.Description>{proxmoxFailureMessage(loadFailure)}</Alert.Description>
            <Alert.Action class="flex flex-wrap gap-2">
              {#if certError}
                <Button
                  data-proxmox-vms-inspect-cert=""
                  size="sm"
                  variant="outline"
                  onclick={inspectCertificateFromVms}
                >
                  {m.proxmox_vms_inspect_cert_hint()}
                </Button>
              {/if}

              <Button size="sm" variant="outline" onclick={retryLoad}>
                {m.proxmox_try_again()}
              </Button>
            </Alert.Action>
          </Alert.Root>
        {:else if request.vms.length === 0}
          <p class="py-4 text-sm text-muted-foreground">{m.proxmox_vms_empty()}</p>
        {:else}
          <div class="flex flex-wrap items-center justify-between gap-2">
            <span class="text-sm text-muted-foreground">
              {m.proxmox_allowed_vm_count({ count: request.selectedIds.length })}
            </span>
            <div class="flex items-center gap-2">
              <Button
                data-proxmox-vms-select-all=""
                disabled={isBusy}
                size="sm"
                type="button"
                variant="outline"
                onclick={selectAll}
              >
                {m.proxmox_vms_select_all()}
              </Button>
              <Button
                data-proxmox-vms-clear-all=""
                disabled={isBusy}
                size="sm"
                type="button"
                variant="outline"
                onclick={clearAll}
              >
                {m.proxmox_vms_clear_all()}
              </Button>
            </div>
          </div>

          <div class="max-h-72 overflow-y-auto rounded-md border">
            <Table.Root>
              <Table.Header>
                <Table.Row>
                  <Table.Head class="w-10"></Table.Head>
                  <Table.Head>{m.proxmox_vms_table_vm()}</Table.Head>
                  <Table.Head>{m.proxmox_vms_table_node()}</Table.Head>
                  <Table.Head>{m.proxmox_vms_table_status()}</Table.Head>
                </Table.Row>
              </Table.Header>

              <Table.Body>
                {#each request.vms as vm (vm.vmid)}
                  <Table.Row data-proxmox-vm-row={vm.vmid}>
                    <Table.Cell>
                      <Checkbox
                        id={`proxmox-vm-${String(vm.vmid)}`}
                        aria-label={m.proxmox_vms_vmid_label({ vmid: String(vm.vmid) })}
                        checked={request.isSelected(vm.vmid)}
                        data-proxmox-vm-checkbox={vm.vmid}
                        disabled={request.saving}
                        onCheckedChange={(checked) => toggleVm(vm.vmid, checked === true)}
                      />
                    </Table.Cell>

                    <Table.Cell>
                      <label class="flex cursor-pointer flex-col" for={`proxmox-vm-${String(vm.vmid)}`}>
                        <span class="font-medium" data-content>{vm.name}</span>
                        <span class="font-mono text-xs text-muted-foreground" data-content dir="ltr">
                          {vm.vmid}
                        </span>
                      </label>
                    </Table.Cell>
                    <Table.Cell class="font-mono text-sm" data-content dir="ltr">{vm.node}</Table.Cell>
                    <Table.Cell>
                      <Badge data-content variant={vm.status === 'running' ? 'secondary' : 'outline'}>
                        {vm.status}
                      </Badge>
                    </Table.Cell>
                  </Table.Row>
                {/each}
              </Table.Body>
            </Table.Root>
          </div>
        {/if}

        {#if request.saving}
          <p class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
            <Spinner aria-hidden="true" />
            <span>{m.proxmox_vms_saving()}</span>
          </p>
        {/if}

        {#if saveFailure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Title>{m.proxmox_vms_failed_title({ server: request.server.name })}</Alert.Title>
            <Alert.Description>{proxmoxFailureMessage(saveFailure)}</Alert.Description>
          </Alert.Root>
        {/if}

        <Dialog.Footer>
          <Button type="button" variant="outline" onclick={closeDialog}>
            {m.proxmox_cancel()}
          </Button>
          {#if request.vms.length > 0}
            <Button data-proxmox-vms-save="" disabled={isBusy} type="button" onclick={saveAllowlist}>
              {m.proxmox_vms_save()}
            </Button>
          {/if}
        </Dialog.Footer>
      </div>
    {/if}
  </Dialog.Content>
</Dialog.Root>
