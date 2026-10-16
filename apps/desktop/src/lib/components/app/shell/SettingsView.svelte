<script lang="ts">
  import SettingsIcon from '@lucide/svelte/icons/settings';

  import { page } from '$app/state';

  import * as Empty from '#lib/components/ui/empty/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import { activeSettingsSection } from '#lib/utils/settings-sections.js';

  import PluginOffState from './PluginOffState.svelte';

  // One page of the settings: the section that the URL names, which the
  // settings sidebar lists beside the others.
  const slots = getShellSlots();

  const active = $derived(activeSettingsSection(slots.settingsSections, page.url));

  function headingId(sectionId: string): string {
    return `settings-section-${sectionId}`;
  }
</script>

{#if active.kind === 'section'}
  <section
    class="mx-auto flex w-full max-w-3xl flex-col gap-5 px-6 py-8"
    aria-labelledby={headingId(active.section.id)}
  >
    <div class="flex flex-col gap-1">
      <h2 id={headingId(active.section.id)} class="text-base font-semibold">{active.section.title()}</h2>
      <p class="text-sm text-muted-foreground">{active.section.description()}</p>
    </div>
    <active.section.component />
  </section>
{:else if active.kind === 'plugin_off'}
  <PluginOffState plugin={active.plugin} />
{:else}
  <Empty.Root class="flex-1">
    <Empty.Header>
      <Empty.Media variant="icon">
        <SettingsIcon />
      </Empty.Media>
      {#if active.kind === 'missing'}
        <Empty.Title>{m.shell_settings_missing_title()}</Empty.Title>
        <Empty.Description>{m.shell_settings_missing_description({ section: active.requested })}</Empty.Description>
      {:else}
        <Empty.Description>{m.shell_settings_empty()}</Empty.Description>
      {/if}
    </Empty.Header>
  </Empty.Root>
{/if}
