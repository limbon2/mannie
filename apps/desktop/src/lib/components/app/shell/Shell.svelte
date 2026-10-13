<script lang="ts">
  import type { Snippet } from 'svelte';

  import { goto } from '$app/navigation';
  import { resolve } from '$app/paths';
  import { page } from '$app/state';

  import * as Sidebar from '#lib/components/ui/sidebar/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import { activeView } from '#lib/utils/views.js';

  import CommandPalette from './CommandPalette.svelte';
  import ShellSidebar from './ShellSidebar.svelte';
  import SidebarToggle from './SidebarToggle.svelte';

  let { children }: { children: Snippet } = $props();

  let paletteOpen = $state(false);

  const slots = getShellSlots();

  const title = $derived(viewTitle());

  function viewTitle(): string {
    if (page.route.id === '/settings') {
      return m.shell_settings_title();
    }
    const active = activeView(slots.views, page.url);
    return active.kind === 'view' ? active.view.title() : m.shell_app_name();
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
      void goto(resolve('/settings'));
    }
  }
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
<Sidebar.Provider>
  <ShellSidebar onFindCommand={() => (paletteOpen = true)} />
  <Sidebar.Inset>
    <header class="flex h-12 shrink-0 items-center gap-2 border-b px-3">
      <SidebarToggle />
      <h1 class="truncate text-sm font-medium">{title}</h1>
    </header>
    <main id="main" class="flex min-h-0 flex-1 flex-col overflow-auto" tabindex="-1">
      {@render children()}
    </main>
  </Sidebar.Inset>
</Sidebar.Provider>
<CommandPalette bind:open={paletteOpen} />
