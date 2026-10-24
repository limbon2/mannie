<script lang="ts">
  import { onMount, type Snippet } from 'svelte';

  import { afterNavigate } from '$app/navigation';
  import { page } from '$app/state';

  import * as Resizable from '#lib/components/ui/resizable/index.js';
  import { SIDEBAR_WIDTH, SIDEBAR_WIDTH_ICON } from '#lib/components/ui/sidebar/constants.js';
  import * as Sidebar from '#lib/components/ui/sidebar/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { activities } from '#lib/state/activities.svelte.js';
  import { pluginStates } from '#lib/state/plugin-states.svelte.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import { MAIN_AREA_ID, sidePanels } from '#lib/state/side-panels.svelte.js';
  import { openSettings } from '#lib/utils/settings-sections.js';
  import { carriesFiles } from '#lib/utils/transfer-files.js';
  import { activeView } from '#lib/utils/views.js';

  import ActivitySidebar from './ActivitySidebar.svelte';
  import CommandPalette from './CommandPalette.svelte';
  import SettingsSidebar from './SettingsSidebar.svelte';
  import ShellSidebar from './ShellSidebar.svelte';
  import SidebarToggle from './SidebarToggle.svelte';
  import SidePanelArea from './SidePanelArea.svelte';

  let { children }: { children: Snippet } = $props();

  // The sidebar holds the activity bar beside its content, so it is wider by
  // the bar (ADR-0045, decision 1). The provider puts this `style` after its
  // own widths, and Svelte sets it through the CSS object model, which the
  // Content Security Policy allows.
  const SIDEBAR_STYLE = `--sidebar-width: calc(${SIDEBAR_WIDTH} + ${SIDEBAR_WIDTH_ICON});`;

  /** Below this width of the main area in pixels, an open side panel covers the view (ADR-0025, decision 2). */
  const NARROW_MAIN_AREA = 720;
  /** The side pane's share of the main area when it first opens, and its limits, in percent. */
  const SIDE_PANE_DEFAULT = 40;
  const SIDE_PANE_MIN = 25;
  const SIDE_PANE_MAX = 65;
  const MAIN_PANE_MIN = 35;
  // PaneForge keeps a layout for each set of panes under their IDs, so the IDs
  // stay the same from one start to the next.
  const MAIN_PANE_ID = 'shell-main-pane';
  const SIDE_PANE_ID = 'shell-side-pane';

  let paletteOpen = $state(false);
  /** The main view's search parameters as the owner last saw them, where the settings lead back to. */
  let mainViewSearch = $state('');
  /** The width of the main area below the header, in pixels; 0 until it is measured. */
  let areaWidth = $state(0);

  const slots = getShellSlots();
  activities.attach(slots.navigation);

  const hasPanels = $derived(sidePanels.state.panels.length > 0);
  // An area that is not measured yet counts as wide, so that a reload with
  // panels does not show them over the view for a moment.
  const narrow = $derived(areaWidth > 0 && areaWidth < NARROW_MAIN_AREA);
  const split = $derived(hasPanels && !narrow);
  const covered = $derived(hasPanels && narrow);

  const onSettings = $derived(page.route.id === '/settings');
  const title = $derived(viewTitle());
  const titleIsContent = $derived(viewTitleIsContent());
  const currentEntry = $derived(activities.current(shownViewId()));
  const selectionIsGone = $derived(selectedEntryIsGone());

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

  /** The ID of the view that the main area shows, or `null` on the settings page and while no view shows. */
  function shownViewId(): string | null {
    if (onSettings) {
      return null;
    }
    const active = activeView(slots.views, page.url);
    return active.kind === 'view' ? active.view.id : null;
  }

  /**
   * Whether the selected entry is gone for good: nothing registered it, or
   * its plugin is off or failed. An entry whose plugin is starting comes
   * back, as at each start of Mannie, so the selection waits for it.
   */
  function selectedEntryIsGone(): boolean {
    const selected = activities.selected;
    if (selected === null) {
      return false;
    }
    const listIsKnown = pluginStates.status === 'ready';
    if (!listIsKnown) {
      return false;
    }
    if (slots.navigation.find(selected) !== undefined) {
      return false;
    }
    const plugin = slots.navigation.pluginOff(selected);
    if (plugin === null) {
      return true;
    }
    const state = pluginStates.view(plugin)?.state;
    const comesBack = state === 'starting' || state === 'on';
    return !comesBack;
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

  /**
   * Files dragged anywhere but on a message box that takes them do nothing:
   * the webview would otherwise open a dropped file in place of Mannie
   * (ADR-0033, decision 7). A message box that takes the files handles the
   * event first.
   */
  function refuseStrayFiles(event: DragEvent): void {
    if (event.defaultPrevented || !carriesFiles(event.dataTransfer)) {
      return;
    }
    event.preventDefault();
    if (event.dataTransfer !== null) {
      event.dataTransfer.dropEffect = 'none';
    }
  }

  // The slots show a plugin's entries while the core reports the plugin as
  // on, so the window follows the plugins' states from its start (ADR-0030,
  // decision 11).
  onMount(() => {
    pluginStates.start();
  });

  // The settings are a page of their own with their own sidebar, which leads
  // back to the main view as the owner left it. Views live at `/` and differ
  // only in their search parameters, and the activity bar keeps them for the
  // entry that is selected (ADR-0045, decision 4).
  afterNavigate(({ to }) => {
    if (to?.route.id !== '/') {
      return;
    }
    mainViewSearch = to.url.search;
    const view = activeView(slots.views, to.url);
    const entry = activities.current(view.kind === 'view' ? view.view.id : null);
    if (entry !== undefined) {
      activities.remember(entry, to.url.search);
    }
  });

  // When the selected entry's plugin stops, the first entry shows again,
  // where the owner left it (ADR-0045, decision 4). Showing it navigates, so
  // it is an effect of the selection being gone. On the settings page, where
  // the owner turns plugins off, the page stays, and its link back leads to
  // the first entry.
  $effect(() => {
    if (!selectionIsGone) {
      return;
    }
    if (!onSettings) {
      void activities.fallBack();
      return;
    }
    const address = activities.fallBackInPlace();
    if (address !== null) {
      mainViewSearch = address;
    }
  });
</script>

<svelte:window ondragover={refuseStrayFiles} ondrop={refuseStrayFiles} onkeydown={handleShortcut} />

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
<Sidebar.Provider class="h-svh" style={SIDEBAR_STYLE}>
  <ShellSidebar current={currentEntry} onFindCommand={() => (paletteOpen = true)} {onSettings}>
    {#if onSettings}
      <SettingsSidebar {mainViewSearch} />
    {:else}
      <ActivitySidebar entry={currentEntry} />
    {/if}
  </ShellSidebar>
  <Sidebar.Inset id={MAIN_AREA_ID} class="min-h-0 overflow-hidden" tabindex={-1}>
    <header class="flex h-12 shrink-0 items-center gap-2 border-b px-3">
      <SidebarToggle />
      <h1 class="truncate text-sm font-medium" data-content={titleIsContent ? '' : undefined}>{title}</h1>
    </header>
    <!-- The view stays in the same pane whether side panels are open or not,
         so opening one never starts the view over. -->
    <div class="relative flex min-h-0 flex-1" bind:clientWidth={areaWidth}>
      <Resizable.PaneGroup class="min-h-0" autoSaveId="mannie.shell.side-panels" direction="horizontal">
        <Resizable.Pane id={MAIN_PANE_ID} class="flex min-h-0 flex-col" minSize={MAIN_PANE_MIN} order={0}>
          <div class="flex min-h-0 flex-1 flex-col overflow-auto" inert={covered}>
            {@render children()}
          </div>
        </Resizable.Pane>
        {#if split}
          <!-- The window splitter pattern: PaneForge gives the handle the main
               pane's share as its value and names that pane as the one it
               controls; the name and the orientation come from here. -->
          <Resizable.Handle aria-label={m.shell_side_panel_resize()} aria-orientation="vertical" withHandle />
          <Resizable.Pane
            id={SIDE_PANE_ID}
            class="flex min-h-0 flex-col"
            defaultSize={SIDE_PANE_DEFAULT}
            maxSize={SIDE_PANE_MAX}
            minSize={SIDE_PANE_MIN}
            order={1}
          >
            <SidePanelArea />
          </Resizable.Pane>
        {/if}
      </Resizable.PaneGroup>
      {#if covered}
        <div class="absolute inset-0 z-10 flex flex-col">
          <SidePanelArea />
        </div>
      {/if}
    </div>
  </Sidebar.Inset>
</Sidebar.Provider>
<CommandPalette bind:open={paletteOpen} />
{#each slots.overlays.entries as overlay (overlay.id)}
  <overlay.component />
{/each}
