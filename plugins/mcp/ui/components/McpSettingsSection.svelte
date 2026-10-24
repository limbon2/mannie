<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import * as Item from '@mannie/plugin-sdk/components/item';
  import { Skeleton } from '@mannie/plugin-sdk/components/skeleton';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';
  import PlugIcon from '@mannie/plugin-sdk/icons/plug';

  import { m } from '../generated/messages.js';
  import { MAX_ENABLED_TOOLS } from '../generated/plugin.js';
  import { mcpActions } from '../state/mcp-actions.svelte.js';
  import { serverList } from '../state/servers.svelte.js';
  import { mcpFailureMessage } from '../utils/mcp-messages.js';

  import ServerRow from './ServerRow.svelte';

  // The MCP section of the settings (plugins/mcp/docs/README.md, settings):
  // the registered servers with their status and actions, how many tools of
  // all servers are on, "Add server" and "Paste JSON", and without servers
  // what adding one does.
</script>

{#if serverList.status === 'error'}
  <Alert.Root variant="destructive">
    <CircleAlertIcon />
    <Alert.Title>{m.mcp_list_failed_title()}</Alert.Title>
    <Alert.Description>{serverList.failure === null ? '' : mcpFailureMessage(serverList.failure)}</Alert.Description>
    <Alert.Action>
      <Button size="sm" variant="outline" onclick={() => serverList.retry()}>{m.mcp_try_again()}</Button>
    </Alert.Action>
  </Alert.Root>
{:else if serverList.status === 'loading'}
  <div class="flex flex-col gap-3" aria-busy="true">
    <span class="sr-only" role="status">{m.mcp_loading()}</span>
    <Skeleton class="h-24 w-full" />
    <Skeleton class="h-24 w-full" />
  </div>
{:else if serverList.servers.length === 0}
  <Empty.Root class="border">
    <Empty.Header>
      <Empty.Media variant="icon">
        <PlugIcon />
      </Empty.Media>
      <Empty.Title>{m.mcp_empty_title()}</Empty.Title>
      <Empty.Description>{m.mcp_empty_description()}</Empty.Description>
    </Empty.Header>
    <Empty.Content class="flex-row flex-wrap justify-center">
      <Button data-mcp-add="" onclick={() => mcpActions.startAdd()}>{m.mcp_add_server()}</Button>
      <Button data-mcp-paste="" variant="outline" onclick={() => mcpActions.startPaste()}>
        {m.mcp_paste_json()}
      </Button>
    </Empty.Content>
  </Empty.Root>
{:else}
  <div class="flex flex-col gap-4">
    <div class="flex flex-wrap items-center gap-2">
      <Button data-mcp-add="" variant="outline" onclick={() => mcpActions.startAdd()}>{m.mcp_add_server()}</Button>
      <Button data-mcp-paste="" variant="outline" onclick={() => mcpActions.startPaste()}>
        {m.mcp_paste_json()}
      </Button>
      <p class="ms-auto text-sm text-muted-foreground" data-mcp-tools-total="">
        {m.mcp_tools_total({ count: formatCount(serverList.enabledTools), limit: formatCount(MAX_ENABLED_TOOLS) })}
      </p>
    </div>
    <Item.Group aria-label={m.mcp_servers_label()}>
      {#each serverList.servers as server (server.id)}
        <ServerRow {server} />
      {/each}
    </Item.Group>
  </div>
{/if}
