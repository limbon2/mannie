<script lang="ts">
  import BotIcon from '@lucide/svelte/icons/bot';
  import SearchIcon from '@lucide/svelte/icons/search';
  import SettingsIcon from '@lucide/svelte/icons/settings';

  import { resolve } from '$app/paths';
  import { page } from '$app/state';

  import { Kbd } from '#lib/components/ui/kbd/index.js';
  import * as Sidebar from '#lib/components/ui/sidebar/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import type { NavigationEntry } from '#lib/types/shell-slots.js';
  import { sortedByOrder } from '#lib/utils/order.js';
  import { activeView, VIEW_PARAMETER } from '#lib/utils/views.js';

  let { onFindCommand }: { onFindCommand: () => void } = $props();

  const slots = getShellSlots();

  const entries = $derived(sortedByOrder(slots.navigation.entries));
  const onSettings = $derived(page.route.id === '/settings');
  const current = $derived(activeView(slots.views, page.url));

  function viewQuery(entry: NavigationEntry): string {
    return new URLSearchParams({ [VIEW_PARAMETER]: entry.view }).toString();
  }

  function isCurrent(entry: NavigationEntry): boolean {
    const showsView = current.kind === 'view' && current.view.id === entry.view;
    return showsView && !onSettings;
  }
</script>

<Sidebar.Root collapsible="icon">
  <Sidebar.Header>
    <div class="flex h-8 items-center gap-2 px-2 font-semibold">
      <BotIcon class="size-4 shrink-0" />
      <span class="truncate group-data-[collapsible=icon]:hidden">{m.shell_app_name()}</span>
    </div>
  </Sidebar.Header>
  <Sidebar.Content>
    <Sidebar.Group>
      <Sidebar.GroupLabel>{m.shell_navigation_label()}</Sidebar.GroupLabel>
      <Sidebar.GroupContent>
        {#if entries.length === 0}
          <p class="px-2 text-xs text-muted-foreground group-data-[collapsible=icon]:hidden">
            {m.shell_navigation_empty()}
          </p>
        {:else}
          <Sidebar.Menu>
            {#each entries as entry (entry.id)}
              <Sidebar.MenuItem>
                <Sidebar.MenuButton isActive={isCurrent(entry)} tooltipContent={entry.label()}>
                  {#snippet child({ props })}
                    <a
                      {...props}
                      aria-current={isCurrent(entry) ? 'page' : undefined}
                      href="{resolve('/')}?{viewQuery(entry)}"
                    >
                      <entry.icon />
                      <span>{entry.label()}</span>
                    </a>
                  {/snippet}
                </Sidebar.MenuButton>
              </Sidebar.MenuItem>
            {/each}
          </Sidebar.Menu>
        {/if}
      </Sidebar.GroupContent>
    </Sidebar.Group>
  </Sidebar.Content>
  <Sidebar.Footer>
    <Sidebar.Menu>
      <Sidebar.MenuItem>
        <Sidebar.MenuButton tooltipContent={m.shell_find_command()} onclick={onFindCommand}>
          <SearchIcon />
          <span>{m.shell_find_command()}</span>
          <Kbd class="ms-auto group-data-[collapsible=icon]:hidden">{m.shell_shortcut_find_command()}</Kbd>
        </Sidebar.MenuButton>
      </Sidebar.MenuItem>
      <Sidebar.MenuItem>
        <Sidebar.MenuButton isActive={onSettings} tooltipContent={m.shell_open_settings()}>
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
