<script lang="ts">
  import * as Alert from '@mannie/plugin-sdk/components/alert';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import * as Item from '@mannie/plugin-sdk/components/item';
  import { Skeleton } from '@mannie/plugin-sdk/components/skeleton';
  import { Switch } from '@mannie/plugin-sdk/components/switch';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import CircleAlertIcon from '@mannie/plugin-sdk/icons/circle-alert';

  import { m } from '../generated/messages.js';
  import { MAX_ENABLED_TOOLS } from '../generated/plugin.js';
  import { mcpActions } from '../state/mcp-actions.svelte.js';
  import { serverList } from '../state/servers.svelte.js';
  import { mcpFailureMessage, toolHintLabel, toolProblemMessage } from '../utils/mcp-messages.js';

  // The tools of a server (plugins/mcp/docs/README.md, Show tools): each
  // with its name for agents, its title and description, what the server
  // says of it, a mark when it arrived after the first listing, and a switch.
  // What the server says is its own claim, so the hints decide nothing. A
  // tool that cannot be offered says why, and its switch stays off. Only the
  // list scrolls, so the title and the buttons stay in view.
  const request = $derived(mcpActions.tools);

  const isEmpty = $derived(request !== null && request.tools.length === 0 && request.failure === null);

  function changeOpen(open: boolean): void {
    const isClosed = request === null;
    if (open || isClosed) {
      return;
    }
    mcpActions.closeTools(request);
  }
</script>

<Dialog.Root bind:open={mcpActions.toolsOpen} onOpenChange={changeOpen}>
  <Dialog.Content class="flex max-h-[90vh] flex-col sm:max-w-2xl" showCloseButton={false}>
    {#if request !== null}
      <div class="flex min-h-0 flex-col gap-4" data-mcp-tools-dialog="">
        <Dialog.Header>
          <Dialog.Title>{m.mcp_tools_title({ server: request.server.name })}</Dialog.Title>
          <Dialog.Description>{m.mcp_tools_description()}</Dialog.Description>
        </Dialog.Header>
        <p class="text-sm text-muted-foreground">
          {m.mcp_tools_total({ count: formatCount(serverList.enabledTools), limit: formatCount(MAX_ENABLED_TOOLS) })}
        </p>
        {#if request.loading}
          <div class="flex flex-col gap-2" aria-busy="true">
            <span class="sr-only" role="status">{m.mcp_tools_loading()}</span>
            <Skeleton class="h-16 w-full" />
            <Skeleton class="h-16 w-full" />
          </div>
        {:else if isEmpty}
          <p class="text-sm text-muted-foreground">{m.mcp_tools_empty()}</p>
        {:else}
          <div class="relative -mx-4 min-h-0 overflow-y-auto px-4 py-1">
            <Item.Group aria-label={m.mcp_tools_title({ server: request.server.name })}>
              {#each request.tools as tool (tool.name)}
                <Item.Root data-mcp-tool={tool.name} role="listitem" size="sm" variant="outline">
                  <Item.Content class="min-w-0 gap-1">
                    <div class="flex min-w-0 flex-wrap items-center gap-2">
                      <Item.Title class="min-w-0 font-mono wrap-anywhere" data-content dir="ltr">{tool.name}</Item.Title
                      >
                      {#if tool.new}
                        <Badge data-mcp-new="" variant="secondary">{m.mcp_tool_new()}</Badge>
                      {/if}
                    </div>
                    {#if tool.title !== null}
                      <p class="text-sm wrap-anywhere" data-content dir="auto">{tool.title}</p>
                    {/if}
                    {#if tool.description !== ''}
                      <p class="line-clamp-4 text-sm wrap-anywhere text-muted-foreground" data-content dir="auto">
                        {tool.description}
                      </p>
                    {/if}
                    <p class="text-xs text-muted-foreground">{m.mcp_tool_model_name({ name: tool.modelName })}</p>
                    {#if tool.hints.length > 0}
                      <div class="flex flex-wrap gap-1">
                        {#each tool.hints as hint (hint)}
                          <Badge data-mcp-hint={hint} variant="outline">{toolHintLabel(hint)}</Badge>
                        {/each}
                      </div>
                    {/if}
                    {#if tool.problem !== null}
                      <p class="text-sm text-destructive" data-mcp-problem={tool.problem}>
                        {toolProblemMessage(tool.problem)}
                      </p>
                    {/if}
                  </Item.Content>
                  <Item.Actions class="self-start">
                    <Switch
                      aria-label={m.mcp_tool_enabled_label({ tool: tool.name })}
                      checked={tool.enabled}
                      disabled={tool.problem !== null || request.switching !== null}
                      onCheckedChange={(checked: boolean) => void mcpActions.setToolEnabled(request, tool, checked)}
                    />
                  </Item.Actions>
                </Item.Root>
              {/each}
            </Item.Group>
          </div>
        {/if}
        {#if request.failure !== null}
          <Alert.Root variant="destructive">
            <CircleAlertIcon />
            <Alert.Title
              >{request.tools.length === 0 ? m.mcp_tools_failed_title() : m.mcp_tool_switch_failed_title()}</Alert.Title
            >
            <Alert.Description>{mcpFailureMessage(request.failure)}</Alert.Description>
            {#if request.tools.length === 0}
              <Alert.Action>
                <Button size="sm" variant="outline" onclick={() => void mcpActions.loadTools(request)}>
                  {m.mcp_try_again()}
                </Button>
              </Alert.Action>
            {/if}
          </Alert.Root>
        {/if}
        <Dialog.Footer>
          <Button
            type="button"
            variant="outline"
            onclick={() => {
              mcpActions.toolsOpen = false;
              changeOpen(false);
            }}
          >
            {m.mcp_close()}
          </Button>
        </Dialog.Footer>
      </div>
    {/if}
  </Dialog.Content>
</Dialog.Root>
