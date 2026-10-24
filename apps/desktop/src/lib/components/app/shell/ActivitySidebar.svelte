<script lang="ts">
  import * as Sidebar from '#lib/components/ui/sidebar/index.js';
  import { activities } from '#lib/state/activities.svelte.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import type { NavigationEntry, SidebarPanel } from '#lib/types/shell-slots.js';
  import { sortedByOrder } from '#lib/utils/order.js';

  // The content sidebar of the selected navigation entry (ADR-0045, decision
  // 3): its label and the sidebar panels that belong to it. A panel that names
  // no entry belongs to the first one, and while no entry is registered, the
  // panels that name none show.
  interface Props {
    /** The entry that the activity bar selects, if any shows. */
    entry: NavigationEntry | undefined;
  }

  let { entry }: Props = $props();

  const slots = getShellSlots();

  const panels = $derived(sortedByOrder(slots.sidebarPanels.entries).filter((panel) => belongs(panel)));

  function belongs(panel: SidebarPanel): boolean {
    if (entry !== undefined) {
      return activities.owns(entry, panel.activity);
    }
    const hasNoEntries = activities.first === undefined;
    const namesNone = panel.activity === undefined;
    return hasNoEntries && namesNone;
  }
</script>

{#if entry !== undefined}
  <Sidebar.Header>
    <div class="flex h-8 items-center px-2 text-sm font-semibold">
      <span class="truncate">{entry.label()}</span>
    </div>
  </Sidebar.Header>
{/if}
<Sidebar.Content>
  {#each panels as panel (panel.id)}
    <panel.component />
  {/each}
</Sidebar.Content>
