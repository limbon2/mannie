<script lang="ts">
  import SettingsIcon from '@lucide/svelte/icons/settings';

  import * as Empty from '#lib/components/ui/empty/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import { sortedByOrder } from '#lib/utils/order.js';

  const slots = getShellSlots();

  const sections = $derived(sortedByOrder(slots.settingsSections.entries));

  function headingId(sectionId: string): string {
    return `settings-section-${sectionId}`;
  }
</script>

{#if sections.length === 0}
  <Empty.Root class="flex-1">
    <Empty.Header>
      <Empty.Media variant="icon">
        <SettingsIcon />
      </Empty.Media>
      <Empty.Description>{m.shell_settings_empty()}</Empty.Description>
    </Empty.Header>
  </Empty.Root>
{:else}
  <div class="mx-auto flex w-full max-w-3xl flex-col gap-10 px-6 py-8">
    {#each sections as section (section.id)}
      <section class="flex flex-col gap-5" aria-labelledby={headingId(section.id)}>
        <div class="flex flex-col gap-1">
          <h2 id={headingId(section.id)} class="text-base font-semibold">{section.title()}</h2>
          <p class="text-sm text-muted-foreground">{section.description()}</p>
        </div>
        <section.component />
      </section>
    {/each}
  </div>
{/if}
