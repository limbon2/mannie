<script lang="ts">
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as DropdownMenu from '@mannie/plugin-sdk/components/dropdown-menu';
  import * as Item from '@mannie/plugin-sdk/components/item';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import EllipsisVerticalIcon from '@mannie/plugin-sdk/icons/ellipsis-vertical';

  import type { ServerStatus, ServerView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { proxmoxActions } from '../state/proxmox-actions.svelte.js';
  import { proxmoxFailureMessage, serverStatusLabel } from '../utils/proxmox-messages.js';

  interface Props {
    readonly server: ServerView;
  }

  let { server }: Props = $props();

  const test = $derived(proxmoxActions.tests[server.id] ?? null);
  const isTesting = $derived(test?.status === 'testing');
  const testResult = $derived(test?.status === 'passed' ? test.result : null);
  const testFailure = $derived(test?.failure ?? null);
  const isPinned = $derived(server.pinnedCertFingerprint !== null);

  function statusVariant(status: ServerStatus): 'destructive' | 'outline' | 'secondary' {
    switch (status) {
      case 'password_missing': {
        return 'destructive';
      }
      case 'ready': {
        return 'secondary';
      }
      case 'no_vms_allowed': {
        return 'outline';
      }
    }
  }

  function chooseVms(): void {
    proxmoxActions.startChooseVms(server);
  }

  function inspectCert(): void {
    proxmoxActions.startInspectCert(server);
  }

  function testConnection(): void {
    void proxmoxActions.testConnection(server);
  }

  function editServer(): void {
    proxmoxActions.startEdit(server);
  }

  function removeServer(): void {
    proxmoxActions.startRemove(server);
  }
</script>

<Item.Root data-proxmox-server={server.name} role="listitem" variant="outline">
  <Item.Content class="min-w-0 gap-2">
    <div class="flex min-w-0 flex-wrap items-center gap-2">
      <Item.Title class="min-w-0 wrap-anywhere" data-content>{server.name}</Item.Title>
      <Badge data-proxmox-status={server.status} variant={statusVariant(server.status)}>
        {serverStatusLabel(server.status)}
      </Badge>
      <Badge data-proxmox-trust={isPinned ? 'pinned' : 'os'} variant="outline">
        {isPinned ? m.proxmox_trust_pinned() : m.proxmox_trust_os()}
      </Badge>
    </div>

    <p class="font-mono text-sm wrap-anywhere text-muted-foreground" data-content dir="ltr">
      {server.username} @ {server.endpoint}
    </p>

    {#if server.note !== ''}
      <p class="text-sm wrap-anywhere whitespace-pre-wrap" data-content dir="auto">{server.note}</p>
    {/if}

    {#if server.pinnedCertFingerprint !== null}
      <p class="font-mono text-xs wrap-anywhere text-muted-foreground" data-content dir="ltr">
        {server.pinnedCertFingerprint}
      </p>
    {/if}

    <div class="flex flex-wrap items-center gap-1.5 text-sm">
      {#if server.allowedVms.length === 0}
        <p class="text-muted-foreground" data-proxmox-allowed-none="">{m.proxmox_allowed_vms_none()}</p>
      {:else}
        <span class="text-muted-foreground" data-proxmox-allowed-count={server.allowedVms.length}>
          {m.proxmox_allowed_vm_count({ count: server.allowedVms.length })}
        </span>
        {#each server.allowedVms as vm (vm.vmid)}
          <Badge data-content data-proxmox-allowed-vm={vm.vmid} variant="secondary">
            {vm.name} ({vm.vmid})
          </Badge>
        {/each}
      {/if}
    </div>

    <div class="text-sm" role="status">
      {#if isTesting}
        <p class="flex items-center gap-2 text-muted-foreground">
          <Spinner aria-hidden="true" />
          <span>{m.proxmox_testing({ server: server.name })}</span>
        </p>
      {:else if testResult !== null}
        <p data-proxmox-test="passed">
          {m.proxmox_test_passed({
            allowed: String(testResult.allowedVmCount),
            server: server.name,
            visible: String(testResult.visibleVmCount),
          })}
        </p>
      {:else if testFailure !== null}
        <p class="text-destructive" data-proxmox-test="failed">
          {m.proxmox_test_failed({ reason: proxmoxFailureMessage(testFailure), server: server.name })}
        </p>
      {/if}
    </div>
  </Item.Content>

  <Item.Actions class="self-start">
    <DropdownMenu.Root>
      <DropdownMenu.Trigger>
        {#snippet child({ props })}
          <Button
            {...props}
            aria-label={m.proxmox_server_actions({ server: server.name })}
            data-proxmox-actions=""
            size="icon-sm"
            variant="ghost"
          >
            <EllipsisVerticalIcon />
          </Button>
        {/snippet}
      </DropdownMenu.Trigger>

      <DropdownMenu.Content align="end">
        <DropdownMenu.Group>
          <DropdownMenu.Item data-proxmox-choose-vms="" disabled={!server.passwordSet} onSelect={chooseVms}>
            {m.proxmox_choose_vms()}
          </DropdownMenu.Item>
          <DropdownMenu.Item data-proxmox-inspect-cert="" onSelect={inspectCert}>
            {m.proxmox_inspect_cert()}
          </DropdownMenu.Item>
          <DropdownMenu.Item
            data-proxmox-test-login=""
            disabled={!server.passwordSet || isTesting}
            onSelect={testConnection}
          >
            {m.proxmox_test_login()}
          </DropdownMenu.Item>
          <DropdownMenu.Item data-proxmox-edit="" onSelect={editServer}>
            {m.proxmox_edit()}
          </DropdownMenu.Item>
        </DropdownMenu.Group>

        <DropdownMenu.Separator />

        <DropdownMenu.Group>
          <DropdownMenu.Item data-proxmox-remove="" variant="destructive" onSelect={removeServer}>
            {m.proxmox_remove()}
          </DropdownMenu.Item>
        </DropdownMenu.Group>
      </DropdownMenu.Content>
    </DropdownMenu.Root>
  </Item.Actions>
</Item.Root>
