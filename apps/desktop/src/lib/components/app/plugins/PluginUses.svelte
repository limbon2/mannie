<script lang="ts">
  import ChevronDownIcon from '@lucide/svelte/icons/chevron-down';

  import { Badge } from '#lib/components/ui/badge/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as Collapsible from '#lib/components/ui/collapsible/index.js';
  import type { PluginUses } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { pluginNativeLabel, pluginServiceLabel } from '#lib/utils/plugins-messages.js';

  // What a plugin's manifest lets it use (docs/features/plugins.md; ADR-0034,
  // decision 13): the services of Mannie, the sensitive ones marked, its
  // entries in the credential store, the commands and channels that its UI
  // may call, the programs that ship with it, the libraries that only it may
  // use, and the system access that its manifest grants outside Mannie's
  // services (ADR-0035, decision 6). The names are the plugin's, so they
  // show as code beside the words. The list starts collapsed below the
  // plugin's description.
  let { open = $bindable(false), uses }: { open?: boolean; uses: PluginUses } = $props();

  const nothing = $derived(
    [uses.services, uses.secrets, uses.commands, uses.channels, uses.programs, uses.crates, uses.native].every(
      (list) => list.length === 0,
    ),
  );
</script>

<Collapsible.Root bind:open>
  <Collapsible.Trigger>
    {#snippet child({ props })}
      <Button {...props} class="-ms-2 text-muted-foreground" data-plugin-uses-trigger="" size="sm" variant="ghost">
        {m.plugins_use_title()}
        <ChevronDownIcon class={['transition-transform', open && 'rotate-180']} data-icon="inline-end" />
      </Button>
    {/snippet}
  </Collapsible.Trigger>
  <Collapsible.Content class="text-sm" data-plugin-uses="">
    {#if nothing}
      <p class="text-muted-foreground">{m.plugins_use_nothing()}</p>
    {:else}
      <dl class="flex flex-col gap-2">
        {#if uses.services.length > 0}
          <div class="flex flex-col gap-1">
            <dt class="font-medium">{m.plugins_use_services()}</dt>
            {#each uses.services as service (service.name)}
              <dd class="flex flex-wrap items-center gap-2 text-muted-foreground" data-plugin-service={service.name}>
                <span>{pluginServiceLabel(service.name)}</span>
                <code class="text-xs" data-content>{service.name}</code>
                {#if service.sensitive}
                  <Badge data-plugin-service-sensitive="" variant="secondary">{m.plugins_use_sensitive()}</Badge>
                {/if}
              </dd>
            {/each}
          </div>
        {/if}
        {#if uses.secrets.length > 0}
          <div class="flex flex-col gap-1">
            <dt class="font-medium">{m.plugins_use_secrets()}</dt>
            {#each uses.secrets as secret (secret)}
              <dd class="text-muted-foreground" data-plugin-secret={secret}>
                <code class="text-xs" data-content>{secret}</code>
              </dd>
            {/each}
          </div>
        {/if}
        {#if uses.commands.length > 0}
          <div class="flex flex-col gap-1">
            <dt class="font-medium">{m.plugins_use_commands()}</dt>
            <dd class="flex flex-wrap gap-x-3 gap-y-1 text-muted-foreground">
              {#each uses.commands as command (command)}
                <code class="text-xs" data-content data-plugin-command={command}>{command}</code>
              {/each}
            </dd>
          </div>
        {/if}
        {#if uses.channels.length > 0}
          <div class="flex flex-col gap-1">
            <dt class="font-medium">{m.plugins_use_channels()}</dt>
            <dd class="flex flex-wrap gap-x-3 gap-y-1 text-muted-foreground">
              {#each uses.channels as channel (channel)}
                <code class="text-xs" data-content data-plugin-channel={channel}>{channel}</code>
              {/each}
            </dd>
          </div>
        {/if}
        {#if uses.programs.length > 0}
          <div class="flex flex-col gap-1">
            <dt class="font-medium">{m.plugins_use_programs()}</dt>
            <dd class="flex flex-wrap gap-x-3 gap-y-1 text-muted-foreground">
              {#each uses.programs as program (program)}
                <code class="text-xs" data-content data-plugin-program={program}>{program}</code>
              {/each}
            </dd>
          </div>
        {/if}
        {#if uses.crates.length > 0}
          <div class="flex flex-col gap-1">
            <dt class="font-medium">{m.plugins_use_crates()}</dt>
            <dd class="flex flex-wrap gap-x-3 gap-y-1 text-muted-foreground">
              {#each uses.crates as name (name)}
                <code class="text-xs" data-content data-plugin-crate={name}>{name}</code>
              {/each}
            </dd>
          </div>
        {/if}
        {#if uses.native.length > 0}
          <div class="flex flex-col gap-1">
            <dt class="font-medium">{m.plugins_use_native()}</dt>
            {#each uses.native as category (category)}
              <dd class="flex flex-wrap items-center gap-2 text-muted-foreground" data-plugin-native={category}>
                <span>{pluginNativeLabel(category)}</span>
                <code class="text-xs" data-content>{category}</code>
              </dd>
            {/each}
          </div>
        {/if}
      </dl>
    {/if}
  </Collapsible.Content>
</Collapsible.Root>
