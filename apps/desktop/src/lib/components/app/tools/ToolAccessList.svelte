<script lang="ts">
  import { Badge } from '#lib/components/ui/badge/index.js';
  import type { Access } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { shellLanguage } from '#lib/utils/highlighter.js';
  import {
    accessModeLabel,
    browserActionLabel,
    databaseHomeText,
    entryCountMessage,
    keepsSnapshot,
    programEffectItems,
    programEffectMessage,
    programRunText,
    proxmoxIdentityText,
    proxmoxVmScopeLabel,
    statementClassLabel,
    toolHintsMessage,
    toolServerEndpointText,
  } from '#lib/utils/tools-messages.js';

  import AccessText from './AccessText.svelte';

  interface Props {
    accesses: readonly Access[];
    /** Whether code blocks offer a Copy button (off inside approvals, T-3). */
    copyable?: boolean;
  }

  // What an operation touches, exactly as the core prepared it: canonical
  // paths with their access modes, a command with its shell and working
  // directory, a command on a remote host with the host, its address and its
  // key (ADR-0036), a run of an installed program with its path, its working
  // directory, the arguments that its plugin fixes and what it does beyond
  // the workspace (ADR-0038), a call of a tool server's tool with its
  // arguments, the server, where it runs and what it says of the tool
  // (ADR-0043), a command typed into a terminal with the terminal, its host,
  // shell, user and folder (ADR-0053, decision 5), a statement on a
  // registered database with its values, what it does and where the database
  // lives, this computer or a worker's host (ADR-0056, decision 7; ADR-0057),
  // a saved network request with its collection and target URL (ADR-0075),
  // a Proxmox VM operation with its server, VM and scope (ADR-0080),
  // or the project that the conversation moves into, with its folder
  // (docs/features/system-tools/tool-activity.md).
  // Paths, commands, statements, values, arguments, names and a server's
  // words are content, and render as text in their own elements (T-3).
  let { accesses, copyable = false }: Props = $props();

  const credentialContent = { 'data-browser-credential-access': '', 'data-content': '' };

  function hasNetworkSummary(summary: string): boolean {
    if (summary === '') {
      return false;
    }
    return summary !== '{}';
  }

  function hasConfigPayload(payload: string): boolean {
    const trimmed = payload.trim();
    if (trimmed === '') {
      return false;
    }
    return trimmed !== '{}';
  }
</script>

<ul class="flex flex-col gap-2">
  {#each accesses as access, index (index)}
    <li class="flex min-w-0 flex-col gap-1.5">
      {#if access.kind === 'path'}
        <div class="flex min-w-0 items-start gap-2">
          <Badge class="mt-px" variant="outline">{accessModeLabel(access.mode)}</Badge>
          <span class="min-w-0 font-mono text-xs wrap-anywhere" data-content>{access.path}</span>
        </div>

        {#if access.entries !== null}
          <p class="text-xs text-muted-foreground">
            {m.tools_access_entries({ entries: entryCountMessage(access.entries) })}
          </p>
        {/if}
      {:else if access.kind === 'project'}
        <p class="text-sm">{m.tools_access_project({ project: access.name })}</p>
        <dl class="flex flex-wrap gap-x-4 gap-y-1 text-xs">
          <div class="flex min-w-0 items-baseline gap-1.5">
            <dt class="shrink-0 text-muted-foreground">{m.tools_access_project_folder()}</dt>
            <dd class="min-w-0 font-mono wrap-anywhere" data-content>{access.folder}</dd>
          </div>
        </dl>
      {:else if access.kind === 'program'}
        <AccessText
          {copyable}
          label={m.tools_access_program_run()}
          language="bash"
          text={programRunText([access.name, ...access.arguments])}
        >
          {#snippet footer()}
            <dl class="flex flex-wrap gap-x-4 gap-y-1" data-program-run="">
              <div class="flex min-w-0 items-baseline gap-1.5">
                <dt class="shrink-0 text-muted-foreground">{m.tools_access_program()}</dt>
                <dd class="min-w-0 font-mono wrap-anywhere text-foreground" data-content>{access.program}</dd>
              </div>
              <div class="flex min-w-0 items-baseline gap-1.5">
                <dt class="shrink-0 text-muted-foreground">{m.tools_access_working_directory()}</dt>
                <dd class="min-w-0 font-mono wrap-anywhere text-foreground" data-content>{access.workingDirectory}</dd>
              </div>
              {#if access.fixed.length > 0}
                <div class="flex min-w-0 items-baseline gap-1.5">
                  <dt class="shrink-0 text-muted-foreground">{m.tools_access_program_fixed()}</dt>
                  <dd class="min-w-0 font-mono wrap-anywhere text-foreground" data-content>
                    {programRunText(access.fixed)}
                  </dd>
                </div>
              {/if}
            </dl>
          {/snippet}
        </AccessText>

        {#if access.effects.length > 0}
          <p class="text-xs font-medium text-muted-foreground">{m.tools_access_program_effects()}</p>
          <ul class="flex list-disc flex-col gap-1 ps-5 text-sm" data-program-effects="">
            {#each access.effects as effect, effectIndex (effectIndex)}
              {const items = $derived(programEffectItems(effect))}
              <li class="wrap-anywhere">
                <span data-content>{programEffectMessage(effect)}</span>
                {#if items !== null}
                  <span class="block font-mono text-xs text-muted-foreground" data-content>{items}</span>
                {/if}
                {#if keepsSnapshot(effect)}
                  <span class="block text-xs text-muted-foreground">{m.tools_effect_snapshot()}</span>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      {:else if access.kind === 'terminal_command'}
        <AccessText
          {copyable}
          label={m.tools_access_command()}
          language={shellLanguage(access.shell)}
          text={access.command}
        >
          {#snippet footer()}
            <dl class="flex flex-wrap gap-x-4 gap-y-1" data-terminal-command="">
              <div class="flex min-w-0 items-baseline gap-1.5">
                <dt class="shrink-0 text-muted-foreground">{m.tools_access_terminal()}</dt>
                <dd class="min-w-0 text-foreground">
                  {access.terminal === null
                    ? m.tools_access_terminal_new()
                    : m.tools_access_terminal_number({ terminal: String(access.terminal) })}
                </dd>
              </div>
              <div class="flex min-w-0 items-baseline gap-1.5">
                <dt class="shrink-0 text-muted-foreground">{m.tools_access_remote_host()}</dt>
                {#if access.host.kind === 'remote'}
                  <dd class="min-w-0 wrap-anywhere text-foreground" data-content dir="auto">{access.host.label}</dd>
                {:else}
                  <dd class="min-w-0 text-foreground">{m.tools_access_terminal_this_machine()}</dd>
                {/if}
              </div>
              {#if access.host.kind === 'remote'}
                <div class="flex min-w-0 items-baseline gap-1.5">
                  <dt class="shrink-0 text-muted-foreground">{m.tools_access_remote_endpoint()}</dt>
                  <dd class="min-w-0 font-mono wrap-anywhere text-foreground" data-content>{access.host.endpoint}</dd>
                </div>
                <div class="flex min-w-0 items-baseline gap-1.5">
                  <dt class="shrink-0 text-muted-foreground">{m.tools_access_remote_identity()}</dt>
                  <dd class="min-w-0 font-mono wrap-anywhere text-foreground" data-content>{access.identity}</dd>
                </div>
              {/if}
              <div class="flex min-w-0 items-baseline gap-1.5">
                <dt class="shrink-0 text-muted-foreground">{m.tools_access_shell()}</dt>
                <dd class="min-w-0 wrap-anywhere text-foreground" data-content dir="auto">{access.shell}</dd>
              </div>
              {#if access.state !== null}
                <div class="flex min-w-0 items-baseline gap-1.5">
                  <dt class="shrink-0 text-muted-foreground">{m.tools_access_terminal_state()}</dt>
                  <dd class="min-w-0 font-mono wrap-anywhere text-foreground" data-content>{access.state}</dd>
                </div>
              {/if}
            </dl>
          {/snippet}
        </AccessText>
        <p class="text-xs text-muted-foreground">{m.tools_access_terminal_as_typed()}</p>
      {:else if access.kind === 'browser_control'}
        <p class="text-sm">{m.tools_access_browser_control()}</p>
      {:else if access.kind === 'browser_action'}
        <p class="text-sm">{browserActionLabel(access.action)}</p>
        <dl class="flex flex-wrap gap-x-4 gap-y-1 text-xs" data-browser-access="">
          <div class="flex min-w-0 items-baseline gap-1.5">
            <dt class="shrink-0 text-muted-foreground">{m.tools_access_browser_site()}</dt>
            <dd class="min-w-0 font-mono wrap-anywhere" data-content>{access.origin}</dd>
          </div>
          {#if access.credential}
            <div class="flex min-w-0 items-baseline gap-1.5">
              <dt class="shrink-0 text-muted-foreground">{m.tools_access_browser_credential()}</dt>
              <dd class="min-w-0 wrap-anywhere" {...credentialContent} dir="auto">
                {access.credential.label}
              </dd>
            </div>
          {/if}
        </dl>
      {:else if access.kind === 'database'}
        <div class="flex min-w-0 items-start gap-2">
          <Badge class="mt-px" variant="outline">{statementClassLabel(access.class)}</Badge>
          <span class="min-w-0 text-sm wrap-anywhere" data-content dir="auto">{access.name}</span>
        </div>
        <AccessText {copyable} label={m.tools_access_database_statement()} language="sql" text={access.statement}>
          {#snippet footer()}
            <dl class="flex flex-wrap gap-x-4 gap-y-1" data-database="">
              <div class="flex min-w-0 items-baseline gap-1.5">
                <dt class="shrink-0 text-muted-foreground">{m.tools_access_database()}</dt>
                <dd class="min-w-0 wrap-anywhere text-foreground" data-content dir="auto">{access.name}</dd>
              </div>
              <div class="flex min-w-0 items-baseline gap-1.5">
                <dt class="shrink-0 text-muted-foreground">{m.tools_access_database_home()}</dt>
                <dd class="min-w-0 wrap-anywhere text-foreground" data-content dir="auto">
                  {databaseHomeText(access.home)}
                </dd>
              </div>
            </dl>
          {/snippet}
        </AccessText>
        {#if access.statementCut ?? false}
          <p class="text-xs text-muted-foreground">{m.tools_access_database_statement_cut()}</p>
        {/if}

        {#if access.parameters.length > 0}
          <p class="text-xs font-medium text-muted-foreground">{m.tools_access_database_values()}</p>
          <dl class="flex flex-wrap gap-x-4 gap-y-1 text-xs" data-database-values="">
            {#each access.parameters as value, valueIndex (valueIndex)}
              <div class="flex min-w-0 items-baseline gap-1.5">
                <dt class="shrink-0 font-mono text-muted-foreground">?{valueIndex + 1}</dt>
                <dd class="min-w-0 font-mono wrap-anywhere" data-content>{value}</dd>
              </div>
            {/each}
          </dl>
        {/if}

        {#if access.parameterCount > access.parameters.length}
          <p class="text-xs text-muted-foreground">
            {m.tools_access_database_values_more({ count: access.parameterCount - access.parameters.length })}
          </p>
        {/if}
      {:else if access.kind === 'external_tool'}
        <p class="text-sm wrap-anywhere" data-content dir="auto">
          {m.tools_access_external_tool({ server: access.label, tool: access.tool })}
        </p>
        <AccessText {copyable} label={m.tools_access_external_arguments()} language="json" text={access.arguments}>
          {#snippet footer()}
            <dl class="flex flex-wrap gap-x-4 gap-y-1" data-external-tool="">
              {#if access.title !== null}
                <div class="flex min-w-0 items-baseline gap-1.5">
                  <dt class="shrink-0 text-muted-foreground">{m.tools_access_external_title()}</dt>
                  <dd class="min-w-0 wrap-anywhere text-foreground" data-content dir="auto">{access.title}</dd>
                </div>
              {/if}
              <div class="flex min-w-0 items-baseline gap-1.5">
                <dt class="shrink-0 text-muted-foreground">{m.tools_access_external_server()}</dt>
                <dd class="min-w-0 wrap-anywhere text-foreground" data-content dir="auto">{access.label}</dd>
              </div>
              <div class="flex min-w-0 items-baseline gap-1.5">
                <dt class="shrink-0 text-muted-foreground">
                  {access.endpoint.kind === 'program'
                    ? m.tools_access_external_program()
                    : m.tools_access_external_url()}
                </dt>
                <dd class="min-w-0 font-mono wrap-anywhere text-foreground" data-content>
                  {toolServerEndpointText(access.endpoint)}
                </dd>
              </div>
            </dl>
          {/snippet}
        </AccessText>
        {#if access.argumentsCut ?? false}
          <p class="text-xs text-muted-foreground">{m.tools_access_external_arguments_cut()}</p>
        {/if}

        {#if access.hints.length > 0}
          <p class="text-xs text-muted-foreground" data-content data-tool-hints="">{toolHintsMessage(access.hints)}</p>
        {/if}
      {:else if access.kind === 'network_request'}
        <div class="flex min-w-0 flex-wrap items-center gap-2">
          <Badge variant="outline">{access.protocol.toUpperCase()}</Badge>
          <Badge variant="secondary">{access.method}</Badge>
          <span class="min-w-0 text-sm wrap-anywhere" data-content dir="auto">{access.requestLabel}</span>
        </div>

        <dl class="flex flex-wrap gap-x-4 gap-y-1 text-xs" data-network-request="">
          <div class="flex min-w-0 items-baseline gap-1.5">
            <dt class="shrink-0 text-muted-foreground">{m.tools_access_network_collection()}</dt>
            <dd class="min-w-0 wrap-anywhere" data-content dir="auto">{access.collectionLabel}</dd>
          </div>
          <div class="flex min-w-0 items-baseline gap-1.5">
            <dt class="shrink-0 text-muted-foreground">{m.tools_access_network_url()}</dt>
            <dd class="min-w-0 font-mono wrap-anywhere" data-content>{access.targetUrl}</dd>
          </div>
        </dl>

        {#if hasNetworkSummary(access.summary)}
          <AccessText {copyable} label={m.tools_access_network_details()} text={access.summary} />
          {#if access.summaryCut ?? false}
            <p class="text-xs text-muted-foreground">{m.tools_access_network_summary_cut()}</p>
          {/if}
        {/if}
      {:else if access.kind === 'proxmox_vm'}
        <div class="flex min-w-0 flex-wrap items-center gap-2">
          <Badge variant="outline">{proxmoxVmScopeLabel(access.scope)}</Badge>
          <Badge data-content variant="secondary">{access.action}</Badge>
          <span class="min-w-0 text-sm wrap-anywhere" data-content dir="auto">
            {m.tools_access_proxmox_vm_value({ node: access.node, vm: access.vmName, vmid: String(access.vmid) })}
          </span>
        </div>

        <dl class="flex flex-wrap gap-x-4 gap-y-1 text-xs" data-proxmox-vm="">
          <div class="flex min-w-0 items-baseline gap-1.5">
            <dt class="shrink-0 text-muted-foreground">{m.tools_access_proxmox_server()}</dt>
            <dd class="min-w-0 wrap-anywhere" data-content dir="auto">{access.serverLabel}</dd>
          </div>
          <div class="flex min-w-0 items-baseline gap-1.5">
            <dt class="shrink-0 text-muted-foreground">{m.tools_access_proxmox_endpoint()}</dt>
            <dd class="min-w-0 font-mono wrap-anywhere" data-content>{access.endpoint}</dd>
          </div>
          <div class="flex min-w-0 items-baseline gap-1.5">
            <dt class="shrink-0 text-muted-foreground">{m.tools_access_proxmox_identity()}</dt>
            <dd class="min-w-0 font-mono wrap-anywhere" data-content>{proxmoxIdentityText(access.identity)}</dd>
          </div>
        </dl>

        {#if access.summary !== ''}
          <AccessText {copyable} label={m.tools_access_proxmox_details()} text={access.summary} />
          {#if access.summaryCut ?? false}
            <p class="text-xs text-muted-foreground">{m.tools_access_proxmox_summary_cut()}</p>
          {/if}
        {/if}
      {:else if access.kind === 'app_config'}
        <div class="flex min-w-0 flex-wrap items-center gap-2">
          <Badge data-content variant="outline">{access.domain}</Badge>
          <Badge data-content variant="secondary">{access.action}</Badge>

          {#if access.securitySensitive ?? false}
            <Badge data-app-config-security="" variant="destructive">
              {m.tools_access_app_config_security()}
            </Badge>
          {/if}
        </div>

        <p class="text-sm font-medium wrap-anywhere" data-content dir="auto">{access.title}</p>

        {#if access.fields.length > 0}
          <dl class="flex flex-wrap gap-x-4 gap-y-1 text-xs" data-app-config="">
            {#each access.fields as field, fieldIndex (fieldIndex)}
              <div class="flex min-w-0 items-baseline gap-1.5">
                <dt class="shrink-0 text-muted-foreground" data-content dir="auto">{field.label}</dt>
                <dd class="min-w-0 font-mono wrap-anywhere" data-content dir="auto">{field.value}</dd>
              </div>
            {/each}
          </dl>
        {/if}

        {#if hasConfigPayload(access.payload)}
          <AccessText {copyable} label={m.tools_access_app_config_details()} language="json" text={access.payload} />

          {#if access.payloadCut ?? false}
            <p class="text-xs text-muted-foreground">{m.tools_access_app_config_payload_cut()}</p>
          {/if}
        {/if}
      {:else}
        <AccessText
          {copyable}
          label={m.tools_access_command()}
          language={access.kind === 'command' ? shellLanguage(access.shell) : 'bash'}
          text={access.command}
        >
          {#snippet footer()}
            {#if access.kind === 'command'}
              <dl class="flex flex-wrap gap-x-4 gap-y-1">
                <div class="flex min-w-0 items-baseline gap-1.5">
                  <dt class="shrink-0 text-muted-foreground">{m.tools_access_working_directory()}</dt>
                  <dd class="min-w-0 font-mono wrap-anywhere text-foreground" data-content>
                    {access.workingDirectory}
                  </dd>
                </div>
                <div class="flex min-w-0 items-baseline gap-1.5">
                  <dt class="shrink-0 text-muted-foreground">{m.tools_access_shell()}</dt>
                  <dd class="min-w-0 font-mono wrap-anywhere text-foreground" data-content>{access.shell}</dd>
                </div>
              </dl>
            {:else}
              <dl class="flex flex-wrap gap-x-4 gap-y-1" data-remote-command="">
                <div class="flex min-w-0 items-baseline gap-1.5">
                  <dt class="shrink-0 text-muted-foreground">{m.tools_access_remote_host()}</dt>
                  <dd class="min-w-0 wrap-anywhere text-foreground" data-content dir="auto">{access.label}</dd>
                </div>
                <div class="flex min-w-0 items-baseline gap-1.5">
                  <dt class="shrink-0 text-muted-foreground">{m.tools_access_remote_endpoint()}</dt>
                  <dd class="min-w-0 font-mono wrap-anywhere text-foreground" data-content>{access.endpoint}</dd>
                </div>
                <div class="flex min-w-0 items-baseline gap-1.5">
                  <dt class="shrink-0 text-muted-foreground">{m.tools_access_remote_identity()}</dt>
                  <dd class="min-w-0 font-mono wrap-anywhere text-foreground" data-content>{access.identity}</dd>
                </div>
              </dl>
            {/if}
          {/snippet}
        </AccessText>
        {#if access.background ?? false}
          <p class="text-xs font-medium" data-background-access="">{m.tools_access_background()}</p>
        {/if}
      {/if}
    </li>
  {/each}
</ul>
