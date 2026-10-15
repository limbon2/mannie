<script lang="ts">
  import XIcon from '@lucide/svelte/icons/x';
  import { Tabs } from 'bits-ui';
  import type { Component } from 'svelte';

  import { m } from '#lib/paraglide/messages.js';
  import type { PanelLabel } from '#lib/state/side-panels.svelte.js';

  // One tab of the side panel area (ADR-0025, decision 3): the panel's icon
  // and label, which show the panel, and the panel's Close inside the tab.
  // Delete closes the focused tab, as in the tabs pattern of WAI-ARIA, so the
  // Close button is for the pointer: it stays out of the tab order, and out
  // of the tab list's accessibility tree, which holds only tabs.
  interface Props {
    /** The icon of the panel's kind. */
    icon: Component;
    /** The tab's label. */
    label: PanelLabel;
    /** Closes the panel. */
    onClose: () => void;
    /** The panel's key, which names the tab. */
    value: string;
  }

  let { icon: Icon, label, onClose, value }: Props = $props();

  function handleKeydown(event: KeyboardEvent): void {
    if (event.key === 'Delete') {
      event.preventDefault();
      onClose();
    }
  }
</script>

<div
  class="group/tab relative flex h-7 max-w-52 min-w-0 shrink-0 items-center rounded-md text-muted-foreground transition-colors hover:bg-muted/60 hover:text-foreground has-focus-visible:ring-[3px] has-focus-visible:ring-ring/50 has-data-[state=active]:bg-muted has-data-[state=active]:text-foreground"
  data-side-panel-tab={value}
>
  <Tabs.Trigger
    class="flex h-full min-w-0 flex-1 items-center gap-1.5 rounded-md ps-2 pe-7 text-sm font-medium outline-none [&_svg]:size-4 [&_svg]:shrink-0"
    aria-keyshortcuts="Delete"
    {value}
    onkeydown={handleKeydown}
  >
    <Icon />
    <span class="truncate" data-content={label.isContent ? '' : undefined}>{label.text}</span>
  </Tabs.Trigger>
  <button
    class="absolute inset-e-1 flex size-5 items-center justify-center rounded-sm text-muted-foreground transition-colors hover:bg-background hover:text-foreground [&_svg]:size-3.5"
    aria-hidden="true"
    data-side-panel-close=""
    tabindex={-1}
    title={m.shell_side_panel_close()}
    type="button"
    onclick={onClose}
  >
    <XIcon />
  </button>
</div>
