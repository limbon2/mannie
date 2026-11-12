<script lang="ts">
  import { Badge } from '#lib/components/ui/badge/index.js';
  import type { Access } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import {
    accessModeLabel,
    browserActionLabel,
    databaseHomeText,
    entryCountMessage,
    keepsSnapshot,
    programEffectItems,
    programEffectMessage,
    programRunText,
    statementClassLabel,
    toolHintsMessage,
    toolServerEndpointText,
  } from '#lib/utils/tools-messages.js';

  import AccessText from './AccessText.svelte';

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
  // or the project that the conversation moves into, with its folder
  // (docs/features/system-tools/tool-activity.md).
  // Paths, commands, statements, values, arguments, names and a server's
  // words are content, and render as text in their own elements (T-3).
  let { accesses }: { accesses: readonly Access[] } = $props();
</script>

<ul class="flex flex-col gap-2">
  {#each accesses as access, index (index)}
    <li class="flex min-w-0 flex-col gap-1">
      {#if access.kind === 'path'}
        <div class="flex min-w-0 items-start gap-2">
          <Badge class="mt-px" variant="outline">{accessModeLabel(access.mode)}</Badge>
          <span class="min-w-0 font-mono text-xs break-all" data-content>{access.path}</span>
        </div>
        {#if access.entries !== null}
          <p class="text-xs text-muted-foreground">
            {m.tools_access_entries({ entries: entryCountMessage(access.entries) })}
          </p>
        {/if}
      {:else if access.kind === 'project'}
        <p class="text-sm">{m.tools_access_project({ project: access.name })}</p>
        <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs">
          <dt class="text-muted-foreground">{m.tools_access_project_folder()}</dt>
          <dd class="font-mono break-all" data-content>{access.folder}</dd>
        </dl>
      {:else if access.kind === 'program'}
        <AccessText label={m.tools_access_program_run()} text={programRunText([access.name, ...access.arguments])} />
        <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs" data-program-run="">
          <dt class="text-muted-foreground">{m.tools_access_program()}</dt>
          <dd class="font-mono break-all" data-content>{access.program}</dd>
          <dt class="text-muted-foreground">{m.tools_access_working_directory()}</dt>
          <dd class="font-mono break-all" data-content>{access.workingDirectory}</dd>
          {#if access.fixed.length > 0}
            <dt class="text-muted-foreground">{m.tools_access_program_fixed()}</dt>
            <dd class="font-mono break-all" data-content>{programRunText(access.fixed)}</dd>
          {/if}
        </dl>
        {#if access.effects.length > 0}
          <p class="text-xs font-medium text-muted-foreground">{m.tools_access_program_effects()}</p>
          <ul class="flex list-disc flex-col gap-1 ps-5 text-sm" data-program-effects="">
            {#each access.effects as effect, effectIndex (effectIndex)}
              {const items = $derived(programEffectItems(effect))}
              <li class="break-all">
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
        <AccessText label={m.tools_access_command()} text={access.command} />
        <p class="text-xs text-muted-foreground">{m.tools_access_terminal_as_typed()}</p>
        <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs" data-terminal-command="">
          <dt class="text-muted-foreground">{m.tools_access_terminal()}</dt>
          <dd>
            {access.terminal === null
              ? m.tools_access_terminal_new()
              : m.tools_access_terminal_number({ terminal: String(access.terminal) })}
          </dd>
          <dt class="text-muted-foreground">{m.tools_access_remote_host()}</dt>
          {#if access.host.kind === 'remote'}
            <dd class="break-all" data-content dir="auto">{access.host.label}</dd>
            <dt class="text-muted-foreground">{m.tools_access_remote_endpoint()}</dt>
            <dd class="font-mono break-all" data-content>{access.host.endpoint}</dd>
            <dt class="text-muted-foreground">{m.tools_access_remote_identity()}</dt>
            <dd class="font-mono break-all" data-content>{access.identity}</dd>
          {:else}
            <dd>{m.tools_access_terminal_this_machine()}</dd>
          {/if}
          <dt class="text-muted-foreground">{m.tools_access_shell()}</dt>
          <dd class="break-all" data-content dir="auto">{access.shell}</dd>
          {#if access.state !== null}
            <dt class="text-muted-foreground">{m.tools_access_terminal_state()}</dt>
            <dd class="font-mono break-all" data-content>{access.state}</dd>
          {/if}
        </dl>
      {:else if access.kind === 'browser_control'}
        <p class="text-sm">{m.tools_access_browser_control()}</p>
      {:else if access.kind === 'browser_action'}
        <p class="text-sm">{browserActionLabel(access.action)}</p>
        <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs" data-browser-access="">
          <dt class="text-muted-foreground">{m.tools_access_browser_site()}</dt>
          <dd class="font-mono break-all" data-content>{access.origin}</dd>
        </dl>
      {:else if access.kind === 'database'}
        <div class="flex min-w-0 items-start gap-2">
          <Badge class="mt-px" variant="outline">{statementClassLabel(access.class)}</Badge>
          <span class="min-w-0 text-sm wrap-anywhere" data-content dir="auto">{access.name}</span>
        </div>
        <AccessText label={m.tools_access_database_statement()} text={access.statement} />
        {#if access.statementCut ?? false}
          <p class="text-xs text-muted-foreground">{m.tools_access_database_statement_cut()}</p>
        {/if}
        {#if access.parameters.length > 0}
          <p class="text-xs font-medium text-muted-foreground">{m.tools_access_database_values()}</p>
          <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs" data-database-values="">
            {#each access.parameters as value, valueIndex (valueIndex)}
              <dt class="font-mono text-muted-foreground">?{valueIndex + 1}</dt>
              <dd class="font-mono break-all" data-content>{value}</dd>
            {/each}
          </dl>
        {/if}
        {#if access.parameterCount > access.parameters.length}
          <p class="text-xs text-muted-foreground">
            {m.tools_access_database_values_more({ count: access.parameterCount - access.parameters.length })}
          </p>
        {/if}
        <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs" data-database="">
          <dt class="text-muted-foreground">{m.tools_access_database()}</dt>
          <dd class="break-all" data-content dir="auto">{access.name}</dd>
          <dt class="text-muted-foreground">{m.tools_access_database_home()}</dt>
          <dd class="break-all" data-content dir="auto">{databaseHomeText(access.home)}</dd>
        </dl>
      {:else if access.kind === 'external_tool'}
        <p class="text-sm wrap-anywhere" data-content dir="auto">
          {m.tools_access_external_tool({ server: access.label, tool: access.tool })}
        </p>
        <AccessText label={m.tools_access_external_arguments()} text={access.arguments} />
        {#if access.argumentsCut ?? false}
          <p class="text-xs text-muted-foreground">{m.tools_access_external_arguments_cut()}</p>
        {/if}
        <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs" data-external-tool="">
          {#if access.title !== null}
            <dt class="text-muted-foreground">{m.tools_access_external_title()}</dt>
            <dd class="break-all" data-content dir="auto">{access.title}</dd>
          {/if}
          <dt class="text-muted-foreground">{m.tools_access_external_server()}</dt>
          <dd class="break-all" data-content dir="auto">{access.label}</dd>
          <dt class="text-muted-foreground">
            {access.endpoint.kind === 'program' ? m.tools_access_external_program() : m.tools_access_external_url()}
          </dt>
          <dd class="font-mono break-all" data-content>{toolServerEndpointText(access.endpoint)}</dd>
        </dl>
        {#if access.hints.length > 0}
          <p class="text-xs text-muted-foreground" data-content data-tool-hints="">{toolHintsMessage(access.hints)}</p>
        {/if}
      {:else}
        <AccessText label={m.tools_access_command()} text={access.command} />
        {#if access.background ?? false}
          <p class="text-xs font-medium" data-background-access="">{m.tools_access_background()}</p>
        {/if}
        {#if access.kind === 'command'}
          <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs">
            <dt class="text-muted-foreground">{m.tools_access_shell()}</dt>
            <dd class="font-mono break-all" data-content>{access.shell}</dd>
            <dt class="text-muted-foreground">{m.tools_access_working_directory()}</dt>
            <dd class="font-mono break-all" data-content>{access.workingDirectory}</dd>
          </dl>
        {:else}
          <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs" data-remote-command="">
            <dt class="text-muted-foreground">{m.tools_access_remote_host()}</dt>
            <dd class="break-all" data-content dir="auto">{access.label}</dd>
            <dt class="text-muted-foreground">{m.tools_access_remote_endpoint()}</dt>
            <dd class="font-mono break-all" data-content>{access.endpoint}</dd>
            <dt class="text-muted-foreground">{m.tools_access_remote_identity()}</dt>
            <dd class="font-mono break-all" data-content>{access.identity}</dd>
          </dl>
        {/if}
      {/if}
    </li>
  {/each}
</ul>
