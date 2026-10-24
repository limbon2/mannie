<script lang="ts">
  import BotIcon from '@lucide/svelte/icons/bot';
  import SearchIcon from '@lucide/svelte/icons/search';
  import SettingsIcon from '@lucide/svelte/icons/settings';

  import { resolve } from '$app/paths';

  import { Kbd } from '#lib/components/ui/kbd/index.js';
  import * as Sidebar from '#lib/components/ui/sidebar/index.js';
  import { useSidebar } from '#lib/components/ui/sidebar/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { activities } from '#lib/state/activities.svelte.js';
  import type { NavigationEntry } from '#lib/types/shell-slots.js';

  // The activity bar (ADR-0045, decision 2): a button for each navigation
  // entry, which shows the entry's sidebar and its view where the owner left
  // it, and at its foot the command palette and the settings. The bar is only
  // icons, so its tooltips show while the sidebar is expanded too, as in
  // shadcn-svelte's block sidebar-09.
  interface Props {
    /** The entry that the bar shows as selected, if any. */
    current: NavigationEntry | undefined;
    /** Opens the command palette. */
    onFindCommand: () => void;
    /** Whether the settings page shows, whose button is then the current one. */
    onSettings: boolean;
  }

  let { current, onFindCommand, onSettings }: Props = $props();

  const sidebar = useSidebar();

  const entries = $derived(activities.entries);
  const tooltipShows = { hidden: false };

  function isCurrent(entry: NavigationEntry): boolean {
    const isSelected = current?.id === entry.id;
    return isSelected && !onSettings;
  }

  function show(event: MouseEvent, entry: NavigationEntry): void {
    // The entry's address is the link's, but selecting the entry comes first,
    // so the main area's address is kept for the entry that shows it.
    event.preventDefault();
    sidebar.setOpen(true);
    void activities.select(entry);
  }
</script>

<Sidebar.Root class="w-[calc(var(--sidebar-width-icon)+1px)]! border-e" collapsible="none" data-shell-sidebar="bar">
  <Sidebar.Header>
    <div class="flex size-8 items-center justify-center">
      <BotIcon class="size-4" aria-hidden="true" />
      <span class="sr-only">{m.shell_app_name()}</span>
    </div>
  </Sidebar.Header>
  <Sidebar.Content>
    <nav aria-label={m.shell_navigation_label()}>
      <Sidebar.Group>
        <Sidebar.GroupContent class="px-1.5 md:px-0">
          <Sidebar.Menu>
            {#each entries as entry (entry.id)}
              <Sidebar.MenuItem>
                <Sidebar.MenuButton
                  class="px-2.5 md:px-2"
                  isActive={isCurrent(entry)}
                  tooltipContent={entry.label()}
                  tooltipContentProps={tooltipShows}
                  onclick={(event: MouseEvent) => show(event, entry)}
                >
                  {#snippet child({ props })}
                    <a
                      {...props}
                      aria-current={isCurrent(entry) ? 'page' : undefined}
                      href="{resolve('/')}{activities.addressOf(entry)}"
                    >
                      <entry.icon />
                      <span>{entry.label()}</span>
                    </a>
                  {/snippet}
                </Sidebar.MenuButton>
              </Sidebar.MenuItem>
            {/each}
          </Sidebar.Menu>
        </Sidebar.GroupContent>
      </Sidebar.Group>
    </nav>
  </Sidebar.Content>
  <Sidebar.Footer>
    <Sidebar.Menu>
      <Sidebar.MenuItem>
        <Sidebar.MenuButton class="px-2.5 md:px-2" tooltipContentProps={tooltipShows} onclick={onFindCommand}>
          {#snippet tooltipContent()}
            <span>{m.shell_find_command()}</span>
            <Kbd>{m.shell_shortcut_find_command()}</Kbd>
          {/snippet}
          <SearchIcon />
          <span>{m.shell_find_command()}</span>
        </Sidebar.MenuButton>
      </Sidebar.MenuItem>
      <Sidebar.MenuItem>
        <Sidebar.MenuButton class="px-2.5 md:px-2" isActive={onSettings} tooltipContentProps={tooltipShows}>
          {#snippet tooltipContent()}
            <span>{m.shell_open_settings()}</span>
            <Kbd>{m.shell_shortcut_open_settings()}</Kbd>
          {/snippet}
          {#snippet child({ props })}
            <a {...props} aria-current={onSettings ? 'page' : undefined} href={resolve('/settings')}>
              <SettingsIcon />
              <span>{m.shell_open_settings()}</span>
            </a>
          {/snippet}
        </Sidebar.MenuButton>
      </Sidebar.MenuItem>
    </Sidebar.Menu>
  </Sidebar.Footer>
</Sidebar.Root>
