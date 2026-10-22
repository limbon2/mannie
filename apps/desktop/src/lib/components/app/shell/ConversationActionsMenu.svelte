<script lang="ts">
  import EllipsisVerticalIcon from '@lucide/svelte/icons/ellipsis-vertical';

  import { Badge } from '#lib/components/ui/badge/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import { m } from '#lib/paraglide/messages.js';
  import { getShellSlots } from '#lib/state/shell-slots.svelte.js';
  import { sidePanels } from '#lib/state/side-panels.svelte.js';
  import type { ConversationControlProps } from '#lib/types/shell-slots.js';
  import { controlsIn, runningInMenu } from '#lib/utils/conversation-controls.js';

  import ConversationControlOutlet from './ConversationControlOutlet.svelte';

  // The conversation's menu at the end of its toolbar (ADR-0042): the entries
  // of the controls whose place is the menu, in their order, behind a button
  // with vertical dots. While the menu is closed, its button counts what the
  // controls report as working, such as running commands (decision 3). The
  // menu's button is the controls' opener, since their entries are gone once
  // the menu closes (decision 5).
  let { backgroundCommands = [], conversation, turns }: ConversationControlProps = $props();

  const slots = getShellSlots();
  const numbers = new Intl.NumberFormat();

  let trigger = $state<HTMLElement | null>(null);
  /** The panel that waited for the focus when the menu opened, so that only a panel that a choice opened keeps it. */
  let focusRequestOnOpen: string | null = null;

  const controls = $derived(controlsIn(slots.conversationControls.entries, 'menu'));
  const running = $derived(
    runningInMenu(slots.conversationControls.entries, { backgroundCommands, conversation, turns }),
  );
  const name = $derived(
    running === 0 ? m.shell_conversation_actions() : m.shell_conversation_actions_running({ count: running }),
  );

  function handleOpenChange(open: boolean): void {
    if (open) {
      focusRequestOnOpen = sidePanels.focusRequest;
    }
  }

  /** A choice that opened a side panel leaves the focus to the panel's heading (ADR-0025). */
  function keepPanelFocus(event: Event): void {
    const request = sidePanels.focusRequest;
    const openedPanel = request !== null && request !== focusRequestOnOpen;
    if (openedPanel) {
      event.preventDefault();
    }
  }
</script>

{#if controls.length > 0}
  <DropdownMenu.Root onOpenChange={handleOpenChange}>
    <DropdownMenu.Trigger bind:ref={trigger}>
      {#snippet child({ props })}
        <Button
          {...props}
          aria-label={name}
          data-conversation-actions=""
          size={running === 0 ? 'icon-sm' : 'sm'}
          variant="ghost"
        >
          <EllipsisVerticalIcon />
          {#if running > 0}
            <Badge variant="secondary">{numbers.format(running)}</Badge>
          {/if}
        </Button>
      {/snippet}
    </DropdownMenu.Trigger>
    <DropdownMenu.Content class="w-60" align="end" onCloseAutoFocus={keepPanelFocus}>
      <ConversationControlOutlet {backgroundCommands} {conversation} opener={trigger} place="menu" {turns} />
      <!-- Shows only while no control rendered an entry before it. -->
      <p class="hidden px-1.5 py-1 text-sm text-muted-foreground first:block">
        {m.shell_conversation_actions_empty()}
      </p>
    </DropdownMenu.Content>
  </DropdownMenu.Root>
{/if}
