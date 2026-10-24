<script lang="ts">
  import type { Snippet } from 'svelte';

  import * as Sidebar from '#lib/components/ui/sidebar/index.js';
  import type { NavigationEntry } from '#lib/types/shell-slots.js';

  import ActivityBar from './ActivityBar.svelte';

  // The window's sidebar (ADR-0045, decision 1): the activity bar and beside
  // it the content of the selected entry or of the settings, as shadcn-svelte's
  // block sidebar-09 composes an icon rail and a second sidebar inside one.
  // Collapsed, the sidebar keeps the bar alone, and the content leaves the
  // page, so that the focus cannot reach it.
  interface Props {
    /** The content beside the bar. */
    children: Snippet;
    /** The entry that the bar shows as selected, if any. */
    current: NavigationEntry | undefined;
    /** Opens the command palette. */
    onFindCommand: () => void;
    /** Whether the settings page shows. */
    onSettings: boolean;
  }

  let { children, current, onFindCommand, onSettings }: Props = $props();
</script>

<Sidebar.Root class="overflow-hidden *:data-[sidebar=sidebar]:flex-row" collapsible="icon">
  <ActivityBar {current} {onFindCommand} {onSettings} />
  <Sidebar.Root
    class="min-w-0 flex-1 group-data-[collapsible=icon]:hidden"
    collapsible="none"
    data-shell-sidebar="content"
  >
    {@render children()}
  </Sidebar.Root>
</Sidebar.Root>
