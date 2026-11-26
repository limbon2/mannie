<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import * as Item from '@mannie/plugin-sdk/components/item';
  import { Skeleton } from '@mannie/plugin-sdk/components/skeleton';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import ServerIcon from '@mannie/plugin-sdk/icons/server';

  import { m } from '../generated/messages.js';
  import { proxmoxActions } from '../state/proxmox-actions.svelte.js';
  import { serverList } from '../state/servers.svelte.js';
  import { proxmoxFailureMessage } from '../utils/proxmox-messages.js';

  import ProxmoxServerRow from './ProxmoxServerRow.svelte';

  function retryLoad(): void {
    serverList.retry();
  }

  function addServer(): void {
    proxmoxActions.startAdd();
  }
</script>

{#if serverList.status === 'error'}
  <Alert.Root variant="destructive">
    <CircleAlertIcon />
    <Alert.Title>{m.proxmox_list_failed_title()}</Alert.Title>
    <Alert.Description>
      {serverList.failure === null ? '' : proxmoxFailureMessage(serverList.failure)}
    </Alert.Description>
    <Alert.Action>
      <Button size="sm" variant="outline" onclick={retryLoad}>{m.proxmox_try_again()}</Button>
    </Alert.Action>
  </Alert.Root>
{:else if serverList.status === 'loading'}
  <div class="flex flex-col gap-3" aria-busy="true">
    <span class="sr-only" role="status">{m.proxmox_loading()}</span>
    <Skeleton class="h-28 w-full" />
    <Skeleton class="h-28 w-full" />
  </div>
{:else if serverList.servers.length === 0}
  <Empty.Root class="border">
    <Empty.Header>
      <Empty.Media variant="icon">
        <ServerIcon />
      </Empty.Media>
      <Empty.Title>{m.proxmox_empty_title()}</Empty.Title>
      <Empty.Description>{m.proxmox_empty_description()}</Empty.Description>
    </Empty.Header>

    <Empty.Content>
      <Button data-proxmox-add="" onclick={addServer}>{m.proxmox_add_server()}</Button>
    </Empty.Content>
  </Empty.Root>
{:else}
  <div class="flex flex-col gap-4">
    <div>
      <Button data-proxmox-add="" variant="outline" onclick={addServer}>{m.proxmox_add_server()}</Button>
    </div>

    <Item.Group aria-label={m.proxmox_servers_label()}>
      {#each serverList.servers as server (server.id)}
        <ProxmoxServerRow {server} />
      {/each}
    </Item.Group>
  </div>
{/if}
