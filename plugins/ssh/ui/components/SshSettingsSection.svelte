<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import * as Item from '@mannie/plugin-sdk/components/item';
  import { Skeleton } from '@mannie/plugin-sdk/components/skeleton';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import ServerIcon from '@mannie/plugin-sdk/icons/server';

  import { m } from '../generated/messages.js';
  import { hostList } from '../state/hosts.svelte.js';
  import { sshActions } from '../state/ssh-actions.svelte.js';
  import { sshFailureMessage } from '../utils/ssh-messages.js';

  import HostRow from './HostRow.svelte';

  // The SSH section of the settings (plugins/ssh/docs/README.md, settings):
  // the registered hosts with their status and actions, "Add SSH host", and
  // without hosts what adding one does.
</script>

{#if hostList.status === 'error'}
  <Alert.Root variant="destructive">
    <CircleAlertIcon />
    <Alert.Title>{m.ssh_list_failed_title()}</Alert.Title>
    <Alert.Description>{hostList.failure === null ? '' : sshFailureMessage(hostList.failure)}</Alert.Description>
    <Alert.Action>
      <Button size="sm" variant="outline" onclick={() => hostList.retry()}>{m.ssh_try_again()}</Button>
    </Alert.Action>
  </Alert.Root>
{:else if hostList.status === 'loading'}
  <div class="flex flex-col gap-3" aria-busy="true">
    <span class="sr-only" role="status">{m.ssh_loading()}</span>
    <Skeleton class="h-24 w-full" />
    <Skeleton class="h-24 w-full" />
  </div>
{:else if hostList.hosts.length === 0}
  <Empty.Root class="border">
    <Empty.Header>
      <Empty.Media variant="icon">
        <ServerIcon />
      </Empty.Media>
      <Empty.Title>{m.ssh_empty_title()}</Empty.Title>
      <Empty.Description>{m.ssh_empty_description()}</Empty.Description>
    </Empty.Header>
    <Empty.Content>
      <Button data-ssh-add="" onclick={() => sshActions.startAdd()}>{m.ssh_add_host()}</Button>
    </Empty.Content>
  </Empty.Root>
{:else}
  <div class="flex flex-col gap-4">
    <div>
      <Button data-ssh-add="" variant="outline" onclick={() => sshActions.startAdd()}>{m.ssh_add_host()}</Button>
    </div>
    <Item.Group aria-label={m.ssh_hosts_label()}>
      {#each hostList.hosts as host (host.id)}
        <HostRow {host} />
      {/each}
    </Item.Group>
  </div>
{/if}
