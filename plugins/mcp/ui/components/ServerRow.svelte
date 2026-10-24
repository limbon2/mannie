<script lang="ts">
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as DropdownMenu from '@mannie/plugin-sdk/components/dropdown-menu';
  import * as Item from '@mannie/plugin-sdk/components/item';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import { Switch } from '@mannie/plugin-sdk/components/switch';
  import { formatCount } from '@mannie/plugin-sdk/format';
  import EllipsisVerticalIcon from '@mannie/plugin-sdk/icons/ellipsis-vertical';
  import { linkFailureMessage } from '@mannie/plugin-sdk/links';

  import type { ServerStatus, ServerView } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { mcpActions, type SignIn } from '../state/mcp-actions.svelte.js';
  import {
    mcpFailureMessage,
    serverFailureMessage,
    serverStatusLabel,
    serverWarningMessage,
  } from '../utils/mcp-messages.js';

  // One server in the MCP settings (plugins/mcp/docs/README.md, settings):
  // its name, its status, what it runs or where it is, its tools and
  // resources, its warnings and the reason it failed, with the owner's
  // actions: the switch, "Sign in…" while the server asks for it, and the
  // rest in a menu. The name and the endpoint are content that the owner
  // wrote, and the endpoint reads left to right in every language.
  let { server }: { server: ServerView } = $props();

  const isOn = $derived(server.enabled);
  const signIn = $derived(mcpActions.signIns[server.id] ?? null);
  const actionFailure = $derived(mcpActions.actionFailures[server.id] ?? null);
  const endpoint = $derived(
    server.transport === 'stdio' ? [server.program ?? '', ...server.arguments].join(' ') : (server.url ?? ''),
  );
  const asksSignIn = $derived(server.status === 'sign_in_needed' || server.status === 'access_needed');
  const canSignIn = $derived(server.oauth && server.enabled);
  const canSignOut = $derived(canSignIn && server.signedIn && !asksSignIn);
  const offersSignIn = $derived(canSignIn && !canSignOut);
  const signInIsBusy = $derived(signIn?.status === 'starting');
  const needsOAuth = $derived(asksSignIn && !server.oauth);

  /** How the badge marks a status: a failure and a sign-in stand out. */
  function statusVariant(status: ServerStatus): 'destructive' | 'outline' | 'secondary' {
    switch (status) {
      case 'failed': {
        return 'destructive';
      }
      case 'connected': {
        return 'secondary';
      }
      case 'access_needed':
      case 'off':
      case 'sign_in_needed':
      case 'starting': {
        return 'outline';
      }
    }
  }

  /** Why a sign-in did not start or open. */
  function signInFailure(failed: SignIn): string {
    if (failed.startFailure !== null) {
      return mcpFailureMessage(failed.startFailure);
    }
    return failed.linkFailure === null ? '' : linkFailureMessage(failed.linkFailure);
  }
</script>

<Item.Root data-mcp-server={server.name} role="listitem" variant="outline">
  <Item.Content class="min-w-0 gap-2">
    <div class="flex min-w-0 flex-wrap items-center gap-2">
      <Item.Title class="min-w-0 wrap-anywhere" data-content>{server.name}</Item.Title>
      <Badge data-mcp-status={server.status} variant={statusVariant(server.status)}>
        {serverStatusLabel(server.status)}
      </Badge>
    </div>
    <p class="font-mono text-sm wrap-anywhere text-muted-foreground" data-content dir="ltr">{endpoint}</p>
    <p class="flex flex-wrap gap-x-3 text-sm text-muted-foreground">
      <span>
        {m.mcp_server_tools({ enabled: formatCount(server.enabledToolCount), total: formatCount(server.toolCount) })}
      </span>
      {#if server.resourceCount > 0}
        <span>{m.mcp_server_resources({ count: server.resourceCount })}</span>
      {/if}
      {#if server.protocolVersion !== null}
        <span>{m.mcp_server_protocol({ version: server.protocolVersion })}</span>
      {/if}
    </p>
    {#if server.failure !== null}
      <p class="text-sm text-destructive" data-mcp-failure={server.failure.code}>
        {serverFailureMessage(server.failure)}
      </p>
    {/if}
    {#if server.missingSecrets.length > 0}
      <p class="text-sm text-destructive">{m.mcp_missing_secrets({ names: server.missingSecrets.join(', ') })}</p>
    {/if}
    {#if needsOAuth}
      <p class="text-sm">{m.mcp_sign_in_oauth_off()}</p>
    {/if}
    {#each server.warnings as warning, index (index)}
      <p class="text-sm text-muted-foreground" data-mcp-warning={warning.code}>{serverWarningMessage(warning)}</p>
    {/each}
    <div class="text-sm" role="status">
      {#if signIn?.status === 'starting'}
        <p class="flex items-center gap-2 text-muted-foreground">
          <Spinner aria-hidden="true" />
          <span>{m.mcp_starting_sign_in()}</span>
        </p>
      {:else if signIn?.status === 'waiting'}
        <p class="flex flex-wrap items-center gap-2" data-mcp-signing-in="">
          <span>{m.mcp_signing_in({ host: signIn.host })}</span>
          <Button size="sm" variant="link" onclick={() => void mcpActions.openSignIn(server.id)}>
            {m.mcp_sign_in_open_again()}
          </Button>
        </p>
      {:else if signIn?.status === 'failed'}
        <p class="text-destructive">{m.mcp_sign_in_failed({ reason: signInFailure(signIn) })}</p>
      {/if}
      {#if actionFailure !== null}
        <p class="text-destructive">
          {m.mcp_server_action_failed({ reason: mcpFailureMessage(actionFailure), server: server.name })}
        </p>
      {/if}
    </div>
  </Item.Content>
  <Item.Actions class="self-start">
    <Switch
      aria-label={m.mcp_server_enabled_label({ server: server.name })}
      checked={isOn}
      data-mcp-enabled=""
      onCheckedChange={(checked: boolean) => void mcpActions.setEnabled(server, checked)}
    />
    {#if offersSignIn}
      <Button
        data-mcp-sign-in=""
        disabled={signInIsBusy}
        size="sm"
        variant="outline"
        onclick={() => void mcpActions.signIn(server)}
      >
        {m.mcp_sign_in()}
      </Button>
    {/if}
    <DropdownMenu.Root>
      <DropdownMenu.Trigger>
        {#snippet child({ props })}
          <Button
            {...props}
            aria-label={m.mcp_server_actions({ server: server.name })}
            data-mcp-actions=""
            size="icon-sm"
            variant="ghost"
          >
            <EllipsisVerticalIcon />
          </Button>
        {/snippet}
      </DropdownMenu.Trigger>
      <DropdownMenu.Content align="end">
        <DropdownMenu.Group>
          {#if canSignOut}
            <DropdownMenu.Item data-mcp-sign-out="" onSelect={() => void mcpActions.signOut(server)}>
              {m.mcp_sign_out()}
            </DropdownMenu.Item>
          {/if}
          <DropdownMenu.Item data-mcp-restart="" disabled={!isOn} onSelect={() => void mcpActions.restart(server)}>
            {m.mcp_restart()}
          </DropdownMenu.Item>
          <DropdownMenu.Item data-mcp-tools="" onSelect={() => mcpActions.startTools(server)}>
            {m.mcp_show_tools()}
          </DropdownMenu.Item>
          <DropdownMenu.Item data-mcp-log="" onSelect={() => mcpActions.startLog(server)}>
            {m.mcp_show_log()}
          </DropdownMenu.Item>
          <DropdownMenu.Item data-mcp-edit="" onSelect={() => mcpActions.startEdit(server)}>
            {m.mcp_edit()}
          </DropdownMenu.Item>
        </DropdownMenu.Group>
        <DropdownMenu.Separator />
        <DropdownMenu.Group>
          <DropdownMenu.Item data-mcp-remove="" variant="destructive" onSelect={() => mcpActions.startRemove(server)}>
            {m.mcp_remove()}
          </DropdownMenu.Item>
        </DropdownMenu.Group>
      </DropdownMenu.Content>
    </DropdownMenu.Root>
  </Item.Actions>
</Item.Root>
