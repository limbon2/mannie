<script lang="ts">
  import type { Snippet } from 'svelte';

  import { afterNavigate } from '$app/navigation';
  import { page } from '$app/state';

  import * as Sidebar from '#lib/components/ui/sidebar/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import { openSettings } from '#lib/utils/settings-sections.js';
  import { activeView } from '#lib/utils/views.js';

  import CommandPalette from './CommandPalette.svelte';
  import SettingsSidebar from './SettingsSidebar.svelte';
  import ShellSidebar from './ShellSidebar.svelte';
  import SidebarToggle from './SidebarToggle.svelte';

  let { children }: { children: Snippet } = $props();

  let paletteOpen = $state(false);
  /** The main view's search parameters as the owner last saw them, where the settings lead back to. */
  let mainViewSearch = $state('');

  const slots = getShellSlots();

  const onSettings = $derived(page.route.id === '/settings');
  const title = $derived(viewTitle());
  const titleIsContent = $derived(viewTitleIsContent());

  function viewTitle(): string {
    if (onSettings) {
      return m.shell_settings_title();
    }
    const active = activeView(slots.views, page.url);
    return active.kind === 'view' ? active.view.title() : m.shell_app_name();
  }

  function viewTitleIsContent(): boolean {
    if (onSettings) {
      return false;
    }
    const active = activeView(slots.views, page.url);
    return active.kind === 'view' ? (active.view.titleIsContent?.() ?? false) : false;
  }

  /** Ctrl+K opens the command palette and Ctrl+, the settings. The sidebar handles Ctrl+B itself. */
  function handleShortcut(event: KeyboardEvent): void {
    const withCommandKey = event.ctrlKey || event.metaKey;
    const key = event.key.toLowerCase();
    if (!withCommandKey) {
      return;
    }
    if (key === 'k') {
      event.preventDefault();
      paletteOpen = !paletteOpen;
    } else if (key === ',') {
      event.preventDefault();
      void openSettings();
    }
  }

  // The settings are a page of their own with their own sidebar, which leads
  // back to the main view as the owner left it. Views live at `/` and differ
  // only in their search parameters.
  afterNavigate(({ to }) => {
    if (to?.route.id === '/') {
      mainViewSearch = to.url.search;
    }
  });
</script>

<svelte:window onkeydown={handleShortcut} />

<svelte:head>
  <title>{title}</title>
</svelte:head>

<a
  class="sr-only z-50 rounded-md bg-background px-3 py-2 text-sm font-medium shadow-sm focus:not-sr-only focus:fixed focus:inset-s-2 focus:top-2"
  href="#main"
>
  {m.shell_skip_to_content()}
</a>
<!-- The window's height is fixed, so that a view can keep parts of itself in
     place, such as a message box below a transcript (ADR-0020, decision 3). -->
<Sidebar.Provider class="h-svh">
  {#if onSettings}
    <SettingsSidebar {mainViewSearch} />
  {:else}
    <ShellSidebar onFindCommand={() => (paletteOpen = true)} />
  {/if}
  <Sidebar.Inset id="main" class="min-h-0 overflow-hidden" tabindex={-1}>
    <header class="flex h-12 shrink-0 items-center gap-2 border-b px-3">
      <SidebarToggle />
      <h1 class="truncate text-sm font-medium" data-content={titleIsContent ? '' : undefined}>{title}</h1>
    </header>
    <div class="flex min-h-0 flex-1 flex-col overflow-auto">
      {@render children()}
    </div>
  </Sidebar.Inset>
</Sidebar.Provider>
<CommandPalette bind:open={paletteOpen} />
{#each slots.overlays.entries as overlay (overlay.id)}
  <overlay.component />
{/each}
