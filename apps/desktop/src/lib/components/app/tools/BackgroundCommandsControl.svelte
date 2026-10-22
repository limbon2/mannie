<script lang="ts">
  import SquareTerminalIcon from '@lucide/svelte/icons/square-terminal';

  import { Badge } from '#lib/components/ui/badge/index.js';
  import { Button } from '#lib/components/ui/button/index.js';
  import * as DropdownMenu from '#lib/components/ui/dropdown-menu/index.js';
  import type { BackgroundCommandView } from '#lib/generated/bindings.js';
  import { m } from '#lib/paraglide/messages.js';
  import { secondClock } from '#lib/state/second-clock.js';
  import { sidePanels } from '#lib/state/side-panels.svelte.js';
  import type { ConversationControlProps } from '#lib/types/shell-slots.js';
  import {
    COMMAND_OUTPUT_PANEL,
    commandLabel,
    commandPanelArgument,
    commandState,
    runningFirst,
  } from '#lib/utils/tools-background.js';

  // The background commands' button in a conversation's toolbar (ADR-0041;
  // docs/features/system-tools/background-commands.md, interface): it says
  // how many commands run, and its menu lists the running commands, then the
  // ended ones, with their states. Choosing one opens its output in a side
  // panel. A worker task's commands show their states only, since their
  // output stays on the worker. Without commands, the toolbar shows nothing
  // of it.
  interface CommandGroup {
    readonly commands: readonly BackgroundCommandView[];
    readonly heading: string;
    readonly id: string;
  }

  let { backgroundCommands = [], conversation }: ConversationControlProps = $props();

  const numbers = new Intl.NumberFormat();

  let open = $state(false);
  let trigger = $state<HTMLElement | null>(null);
  /** Whether the latest choice opened a panel, whose heading then keeps the focus that the menu would give back. */
  let openedPanel = false;

  const split = $derived(runningFirst(backgroundCommands));
  /** The menu's groups that have commands: the ones that run, then the ones that ended. */
  const groups: CommandGroup[] = $derived(
    [
      { commands: split.running, heading: m.tools_background_control_running(), id: 'running' },
      { commands: split.ended, heading: m.tools_background_control_ended(), id: 'ended' },
    ].filter((group) => group.commands.length > 0),
  );
  const name = $derived(
    split.running.length === 0
      ? m.tools_background_control_label()
      : m.tools_background_control_label_running({ count: split.running.length }),
  );
  // Only an open menu reads the clock, so a closed one starts no timer.
  const now = $derived(open ? secondClock.now : 0);

  function openOutput(command: BackgroundCommandView): void {
    openedPanel = true;
    const label = { isContent: true, text: commandLabel(command) };
    const argument = commandPanelArgument(conversation.id, command.number);
    void sidePanels.open(COMMAND_OUTPUT_PANEL, argument, { label, opener: trigger });
  }

  function keepPanelFocus(event: Event): void {
    if (openedPanel) {
      event.preventDefault();
      openedPanel = false;
    }
  }
</script>

{#if backgroundCommands.length > 0}
  <DropdownMenu.Root bind:open>
    <DropdownMenu.Trigger bind:ref={trigger}>
      {#snippet child({ props })}
        <Button {...props} aria-label={name} data-background-commands="" size="sm" variant="ghost">
          <SquareTerminalIcon data-icon="inline-start" />
          {m.tools_background_control_label()}
          {#if split.running.length > 0}
            <Badge variant="secondary">{numbers.format(split.running.length)}</Badge>
          {/if}
        </Button>
      {/snippet}
    </DropdownMenu.Trigger>
    <DropdownMenu.Content class="w-80" align="end" onCloseAutoFocus={keepPanelFocus}>
      {#each groups as group, index (group.id)}
        {#if index > 0}
          <DropdownMenu.Separator />
        {/if}
        <DropdownMenu.Group>
          <DropdownMenu.GroupHeading>{group.heading}</DropdownMenu.GroupHeading>
          {#each group.commands as command (command.number)}
            <DropdownMenu.Item
              disabled={command.onWorker}
              textValue={commandLabel(command)}
              onSelect={() => openOutput(command)}
            >
              <span class="min-w-0 flex-1 truncate" data-content dir="auto">{commandLabel(command)}</span>
              <span class="shrink-0 text-xs text-muted-foreground">{commandState(command, now)}</span>
            </DropdownMenu.Item>
          {/each}
        </DropdownMenu.Group>
      {/each}
    </DropdownMenu.Content>
  </DropdownMenu.Root>
{/if}
