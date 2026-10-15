<script lang="ts">
  import PanelRightIcon from '@lucide/svelte/icons/panel-right';
  import { Tabs } from 'bits-ui';
  import { tick } from 'svelte';
  import type { Attachment } from 'svelte/attachments';

  import { m } from '#lib/paraglide/messages.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import { type PanelLabel, sidePanels } from '#lib/state/side-panels.svelte.js';
  import type { OpenPanel } from '#lib/utils/side-panels.js';

  import SidePanelTab from './SidePanelTab.svelte';
  import UnknownSidePanel from './UnknownSidePanel.svelte';

  // The side panel area (ADR-0025): a tab for each open panel, with the
  // panel's Close inside it, and the panels' bodies, which stay mounted while
  // their tab is open so they keep their state. The area is not modal: the
  // view beside it keeps working.

  const slots = getShellSlots();

  let tabList = $state<HTMLElement | null>(null);

  const open = $derived(sidePanels.state);

  function labelOf(panel: OpenPanel): PanelLabel {
    const named = sidePanels.label(panel.key);
    if (named !== undefined) {
      return named;
    }
    const kind = slots.sidePanels.find(panel.kind);
    const text = kind === undefined ? m.shell_side_panel_unknown_label() : kind.defaultLabel();
    return { isContent: false, text };
  }

  function show(key: string): void {
    void sidePanels.show(key);
  }

  /**
   * Closes a panel from its tab. While other panels stay open, the focus
   * moves to the tab that shows next, as after Delete in the tabs pattern of
   * WAI-ARIA; closing the last panel returns the focus to its opener.
   */
  async function closeTab(key: string): Promise<void> {
    const isLast = open.panels.length === 1;
    if (isLast) {
      await sidePanels.close(key);
      return;
    }
    await sidePanels.close(key, { returnFocus: false });
    await tick();
    tabList?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')?.focus();
  }

  /**
   * Moves the focus to a panel's heading once the panel shows, when a
   * control opened it. A reload requests no focus, so it moves none.
   */
  function focusWhenOpened(key: string): Attachment<HTMLElement> {
    return (heading) => {
      const isRequested = sidePanels.focusRequest === key;
      const isShown = open.shown === key;
      if (isRequested && isShown) {
        heading.focus();
        sidePanels.focusRequest = null;
      }
    };
  }
</script>

<section class="flex h-full min-h-0 flex-col bg-background" aria-label={m.shell_side_panel_label()} data-side-panels="">
  <Tabs.Root class="flex min-h-0 flex-1 flex-col" value={open.shown ?? ''} onValueChange={show}>
    <Tabs.List
      class="flex h-10 shrink-0 items-center gap-1 overflow-x-auto overflow-y-hidden border-b px-2"
      aria-label={m.shell_side_panel_tabs()}
      bind:ref={tabList}
    >
      {#each open.panels as panel (panel.key)}
        <SidePanelTab
          icon={slots.sidePanels.find(panel.kind)?.icon ?? PanelRightIcon}
          label={labelOf(panel)}
          value={panel.key}
          onClose={() => void closeTab(panel.key)}
        />
      {/each}
    </Tabs.List>
    {#each open.panels as panel (panel.key)}
      {const kind = $derived(slots.sidePanels.find(panel.kind))}
      {const label = $derived(labelOf(panel))}
      <Tabs.Content class="flex min-h-0 flex-1 flex-col outline-none" tabindex={-1} value={panel.key}>
        <h2
          class="shrink-0 truncate px-3 pt-3 text-sm font-medium outline-none focus-visible:ring-2 focus-visible:ring-ring"
          {@attach focusWhenOpened(panel.key)}
          data-content={label.isContent ? '' : undefined}
          tabindex="-1"
        >
          {label.text}
        </h2>
        <div class="flex min-h-0 flex-1 flex-col">
          {#if kind}
            <kind.component
              argument={panel.argument}
              close={() => void sidePanels.close(panel.key)}
              setLabel={(text, isContent) => {
                sidePanels.setLabel(panel.key, { isContent, text });
              }}
            />
          {:else}
            <UnknownSidePanel close={() => void sidePanels.close(panel.key)} kind={panel.kind} />
          {/if}
        </div>
      </Tabs.Content>
    {/each}
  </Tabs.Root>
</section>
