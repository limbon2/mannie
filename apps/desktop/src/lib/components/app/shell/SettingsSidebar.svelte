<script lang="ts">
  import ArrowLeftIcon from '@lucide/svelte/icons/arrow-left';

  import { resolve } from '$app/paths';
  import { page } from '$app/state';

  import * as Sidebar from '#lib/components/ui/sidebar/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import type { SettingsSection } from '#lib/types/shell-slots.js';
  import { sortedByOrder } from '#lib/utils/order.js';
  import { activeSettingsSection, settingsSectionQuery } from '#lib/utils/settings-sections.js';

  // The content of the sidebar on the settings page, in place of the selected
  // entry's (ADR-0045, decision 5): the way back to the main view as the
  // owner left it, and a page per section.
  interface Props {
    /** The main view's search parameters when the owner left it, with the `?`, or empty. */
    mainViewSearch: string;
  }

  let { mainViewSearch }: Props = $props();

  const slots = getShellSlots();

  const sections = $derived(sortedByOrder(slots.settingsSections.entries));
  const active = $derived(activeSettingsSection(slots.settingsSections, page.url));

  function isCurrent(section: SettingsSection): boolean {
    const isShown = active.kind === 'section' && active.section.id === section.id;
    return isShown;
  }
</script>

<Sidebar.Header>
  <Sidebar.Menu>
    <Sidebar.MenuItem>
      <Sidebar.MenuButton>
        {#snippet child({ props })}
          <a {...props} href="{resolve('/')}{mainViewSearch}">
            <ArrowLeftIcon />
            <span>{m.shell_settings_back()}</span>
          </a>
        {/snippet}
      </Sidebar.MenuButton>
    </Sidebar.MenuItem>
  </Sidebar.Menu>
</Sidebar.Header>
<Sidebar.Content>
  <Sidebar.Group>
    <Sidebar.GroupLabel>{m.shell_settings_title()}</Sidebar.GroupLabel>
    <Sidebar.GroupContent>
      <Sidebar.Menu>
        {#each sections as section (section.id)}
          <Sidebar.MenuItem>
            <Sidebar.MenuButton isActive={isCurrent(section)}>
              {#snippet child({ props })}
                <a
                  {...props}
                  aria-current={isCurrent(section) ? 'page' : undefined}
                  href="{resolve('/settings')}?{settingsSectionQuery(section.id)}"
                >
                  <section.icon />
                  <span>{section.title()}</span>
                </a>
              {/snippet}
            </Sidebar.MenuButton>
          </Sidebar.MenuItem>
        {/each}
      </Sidebar.Menu>
    </Sidebar.GroupContent>
  </Sidebar.Group>
</Sidebar.Content>
