<script lang="ts">
  import { Badge } from '#lib/components/ui/badge/index.js';
  import type { Access } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import {
    accessModeLabel,
    entryCountMessage,
    keepsSnapshot,
    programEffectItems,
    programEffectMessage,
    programRunText,
  } from '#lib/utils/tools-messages.js';

  import AccessText from './AccessText.svelte';

  // What an operation touches, exactly as the core prepared it: canonical
  // paths with their access modes, a command with its shell and working
  // directory, a command on a remote host with the host, its address and its
  // key (ADR-0036), a run of an installed program with its path, its working
  // directory, the arguments that its plugin fixes and what it does beyond
  // the workspace (ADR-0038), or the project that the conversation moves
  // into, with its folder (docs/features/system-tools/tool-activity.md).
  // Paths, commands, arguments and names are content, and render as text in
  // their own elements (T-3).
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
      {:else}
        <AccessText label={m.tools_access_command()} text={access.command} />
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
