<script lang="ts">
  import * as AlertDialog from '@mannie/plugin-sdk/components/alert-dialog';
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as DropdownMenu from '@mannie/plugin-sdk/components/dropdown-menu';
  import SquareTerminalIcon from '@mannie/plugin-sdk/icons/square-terminal';
  import { openSidePanel } from '@mannie/plugin-sdk/navigation';
  import type { ConversationControlProps } from '@mannie/plugin-sdk/slots';

  import type { HostChoice, TerminalSummary } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import { terminals } from '../state/terminals.svelte.js';
  import type { TerminalFailure } from '../types/terminal.js';
  import { phaseLabel, terminalFailureMessage, terminalName, unavailableReason } from '../utils/terminal-messages.js';
  import { panelArgument, TERMINAL_PANEL } from '../utils/terminal-panel.js';

  // The Terminal button in a conversation's toolbar (plugins/terminal/docs/
  // README.md, opening and using terminals): a menu of the conversation's
  // terminals with what each does, and New terminal on this computer, in one
  // of its shells, or on a host that a plugin offers. A host that opens no
  // terminals now is disabled, with its reason. The button counts the
  // terminals that run a command.
  let { conversation }: ConversationControlProps = $props();

  let trigger = $state<HTMLElement | null>(null);
  let opening = $state(false);
  let failure = $state<TerminalFailure | null>(null);

  const list = $derived(terminals.of(conversation.id));
  const running = $derived(list.filter((terminal) => terminal.phase === 'running').length);
  const shells = $derived(terminals.hosts?.shells ?? []);
  const hosts = $derived(terminals.hosts?.hosts ?? []);

  function show(terminal: TerminalSummary): void {
    void openSidePanel(TERMINAL_PANEL, panelArgument({ conversationId: conversation.id, terminal: terminal.number }), {
      opener: trigger,
    });
  }

  async function openNew(host: HostChoice): Promise<void> {
    opening = true;
    failure = await terminals.open(conversation.id, host, trigger);
    opening = false;
  }

  $effect(() => terminals.watch(conversation.id));
</script>

<DropdownMenu.Root>
  <DropdownMenu.Trigger>
    {#snippet child({ props })}
      <Button
        {...props}
        aria-label={running > 0 ? m.terminal_control_label({ running }) : m.terminal_control_title()}
        data-terminal-control=""
        disabled={opening}
        size="sm"
        variant="ghost"
        bind:ref={trigger}
      >
        <SquareTerminalIcon data-icon="inline-start" />
        <span>{m.terminal_control_title()}</span>
        {#if running > 0}
          <Badge variant="secondary">{running}</Badge>
        {/if}
      </Button>
    {/snippet}
  </DropdownMenu.Trigger>
  <DropdownMenu.Content class="w-72" align="end">
    {#if list.length > 0}
      <DropdownMenu.Group>
        <DropdownMenu.Label>{m.terminal_control_terminals()}</DropdownMenu.Label>
        {#each list as terminal (terminal.number)}
          <DropdownMenu.Item onSelect={() => show(terminal)}>
            <SquareTerminalIcon />
            <span class="flex min-w-0 flex-1 flex-col">
              <span class="truncate" data-content>{terminalName(terminal)}</span>
              <span class="text-xs text-muted-foreground">{phaseLabel(terminal.phase)}</span>
            </span>
          </DropdownMenu.Item>
        {/each}
      </DropdownMenu.Group>
      <DropdownMenu.Separator />
    {/if}
    <DropdownMenu.Sub>
      <DropdownMenu.SubTrigger>{m.terminal_control_new()}</DropdownMenu.SubTrigger>
      <DropdownMenu.SubContent class="w-64">
        <DropdownMenu.Group>
          <DropdownMenu.Label>{m.terminal_this_computer()}</DropdownMenu.Label>
          {#if shells.length === 0}
            <DropdownMenu.Item disabled>{m.terminal_control_no_shells()}</DropdownMenu.Item>
          {/if}
          {#each shells as shell (shell.program)}
            <DropdownMenu.Item onSelect={() => void openNew({ kind: 'this_computer', shell: shell.program })}>
              <span class="truncate" data-content>{shell.name}</span>
              {#if shell.default}
                <span class="ms-auto text-xs text-muted-foreground">{m.terminal_control_default()}</span>
              {/if}
            </DropdownMenu.Item>
          {/each}
        </DropdownMenu.Group>
        {#if hosts.length > 0}
          <DropdownMenu.Separator />
          <DropdownMenu.Group>
            <DropdownMenu.Label>{m.terminal_control_hosts()}</DropdownMenu.Label>
            {#each hosts as host (`${host.plugin}/${host.key}`)}
              {const unavailable = host.state.kind === 'unavailable' ? host.state.code : null}
              <DropdownMenu.Item
                disabled={unavailable !== null}
                onSelect={() => void openNew({ key: host.key, kind: 'plugin', plugin: host.plugin })}
              >
                <span class="flex min-w-0 flex-1 flex-col">
                  <span class="truncate" data-content>{host.label}</span>
                  <span class="truncate font-mono text-xs text-muted-foreground" data-content>
                    {unavailable === null ? host.endpoint : unavailableReason(unavailable)}
                  </span>
                </span>
              </DropdownMenu.Item>
            {/each}
          </DropdownMenu.Group>
        {/if}
      </DropdownMenu.SubContent>
    </DropdownMenu.Sub>
  </DropdownMenu.Content>
</DropdownMenu.Root>

<AlertDialog.Root
  bind:open={
    () => failure !== null,
    (open) => {
      if (!open) {
        failure = null;
      }
    }
  }
>
  <AlertDialog.Content>
    <AlertDialog.Header>
      <AlertDialog.Title>{m.terminal_open_failed_title()}</AlertDialog.Title>
      <AlertDialog.Description>{failure === null ? '' : terminalFailureMessage(failure)}</AlertDialog.Description>
    </AlertDialog.Header>
    <AlertDialog.Footer>
      <AlertDialog.Cancel>{m.terminal_dismiss()}</AlertDialog.Cancel>
    </AlertDialog.Footer>
  </AlertDialog.Content>
</AlertDialog.Root>
