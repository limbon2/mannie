<script lang="ts">
  import { Badge } from '#lib/components/ui/badge/index.js';
  import type { Access } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { accessModeLabel, entryCountMessage } from '#lib/utils/tools-messages.js';

  // What an operation touches, exactly as the core prepared it: canonical
  // paths with their access modes, a command with its shell and working
  // directory, a command on a remote host with the host, its address and its
  // key (ADR-0036), or the project that the conversation moves into, with its
  // folder (docs/features/system-tools/tool-activity.md). Paths, commands and
  // names are content, and render as text in their own elements (T-3).
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
      {:else}
        <!-- Focusable, so that the owner can scroll a long command from the keyboard (WCAG 2.1.1). -->
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <pre
          class="max-h-48 overflow-auto rounded-md bg-muted px-2 py-1.5 font-mono text-xs break-all whitespace-pre-wrap outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-label={m.tools_access_command()}
          data-content
          role="group"
          tabindex="0">{access.command}</pre>
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
