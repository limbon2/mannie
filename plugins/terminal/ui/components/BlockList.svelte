<script lang="ts">
  import { Badge } from '@mannie/plugin-sdk/components/badge';
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as DropdownMenu from '@mannie/plugin-sdk/components/dropdown-menu';
  import * as Empty from '@mannie/plugin-sdk/components/empty';
  import { formatTime } from '@mannie/plugin-sdk/format';
  import BotIcon from '@mannie/plugin-sdk/icons/bot';
  import EllipsisIcon from '@mannie/plugin-sdk/icons/ellipsis';
  import UserIcon from '@mannie/plugin-sdk/icons/user';
  import { call } from '@mannie/plugin-sdk/ipc';
  import { mention } from '@mannie/plugin-sdk/message-box';

  import { type BlockView, commands } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import type { TerminalAddress, TerminalFailure } from '../types/terminal.js';
  import { terminalFailureMessage } from '../utils/terminal-messages.js';

  import BlockOutputDialog from './BlockOutputDialog.svelte';

  // The commands of a terminal (plugins/terminal/docs/README.md, the command
  // list), the latest first: who ran each, its exit code and its time, with
  // Show, Mention in message, Copy command, Copy output and Show output as
  // text. It serves the keyboard and screen readers, which the terminal's
  // canvas does not.
  interface Props {
    readonly address: TerminalAddress;
    readonly blocks: readonly BlockView[];
    /** Scrolls the terminal to a command and marks it. */
    readonly onShow: (block: number) => void;
  }

  let { address, blocks, onShow }: Props = $props();

  let shown = $state<BlockView | null>(null);
  let failure = $state<TerminalFailure | null>(null);
  let notice = $state<string | null>(null);

  const latestFirst = $derived([...blocks].reverse());

  /** Whether a command ended with an exit code other than 0. */
  function failed(block: BlockView): boolean {
    if (block.exitCode === null) {
      return false;
    }
    return block.exitCode !== 0;
  }

  function exitLabel(block: BlockView): string {
    if (block.running) {
      return m.terminal_block_running();
    }
    return block.exitCode === null ? m.terminal_block_ended() : m.terminal_block_exit({ code: String(block.exitCode) });
  }

  async function mentionBlock(block: BlockView): Promise<void> {
    const outcome = await call(() =>
      commands.mentionBlock({
        block: block.number,
        conversationId: address.conversationId,
        terminal: address.terminal,
      }),
    );
    if (!outcome.ok) {
      failure = outcome.failure;
      return;
    }
    const added = mention(address.conversationId, { entry: outcome.data, provider: 'terminal' });
    notice = added === 'added' ? null : m.terminal_mention_not_added();
  }

  async function copyOutput(block: BlockView): Promise<void> {
    const outcome = await call(() =>
      commands.blockText({ block: block.number, conversationId: address.conversationId, terminal: address.terminal }),
    );
    if (!outcome.ok) {
      failure = outcome.failure;
      return;
    }
    await navigator.clipboard.writeText(outcome.data.lines.join('\n'));
    notice = m.terminal_copied();
  }

  async function copyCommand(block: BlockView): Promise<void> {
    await navigator.clipboard.writeText(block.command);
    notice = m.terminal_copied();
  }
</script>

<section class="flex flex-col gap-1 p-2" aria-label={m.terminal_commands()}>
  {#if failure !== null}
    <p class="text-xs text-destructive" role="alert">{terminalFailureMessage(failure)}</p>
  {/if}
  {#if notice !== null}
    <p class="text-xs text-muted-foreground" role="status">{notice}</p>
  {/if}
  {#if latestFirst.length === 0}
    <Empty.Root class="p-4">
      <Empty.Header>
        <Empty.Title>{m.terminal_commands_empty()}</Empty.Title>
        <Empty.Description>{m.terminal_commands_empty_description()}</Empty.Description>
      </Empty.Header>
    </Empty.Root>
  {:else}
    <ul class="flex flex-col gap-1">
      {#each latestFirst as block (block.number)}
        <li class="flex items-start gap-2 rounded-md border p-2" data-block={block.number}>
          {#if block.author === 'agent'}
            <BotIcon class="mt-0.5 size-4 shrink-0" aria-label={m.terminal_block_by_agent()} />
          {:else}
            <UserIcon class="mt-0.5 size-4 shrink-0" aria-label={m.terminal_block_by_owner()} />
          {/if}
          <button
            class="flex min-w-0 flex-1 flex-col text-start focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
            type="button"
            onclick={() => onShow(block.number)}
          >
            <span class="truncate font-mono text-xs" data-content dir="ltr"
              >{block.command || m.terminal_block_no_command()}</span
            >
            <span class="flex items-center gap-1 text-xs text-muted-foreground">
              <Badge variant={failed(block) ? 'destructive' : 'secondary'}>
                {exitLabel(block)}
              </Badge>
              <time datetime={new Date(block.startedAt).toISOString()}>{formatTime(block.startedAt)}</time>
            </span>
          </button>
          <DropdownMenu.Root>
            <DropdownMenu.Trigger>
              {#snippet child({ props })}
                <Button
                  {...props}
                  aria-label={m.terminal_block_actions({ number: block.number })}
                  size="icon-sm"
                  variant="ghost"
                >
                  <EllipsisIcon />
                </Button>
              {/snippet}
            </DropdownMenu.Trigger>
            <DropdownMenu.Content align="end">
              <DropdownMenu.Item onSelect={() => onShow(block.number)}>{m.terminal_block_show()}</DropdownMenu.Item>
              <DropdownMenu.Item onSelect={() => void mentionBlock(block)}
                >{m.terminal_block_mention()}</DropdownMenu.Item
              >
              <DropdownMenu.Item onSelect={() => void copyCommand(block)}
                >{m.terminal_block_copy_command()}</DropdownMenu.Item
              >
              <DropdownMenu.Item onSelect={() => void copyOutput(block)}
                >{m.terminal_block_copy_output()}</DropdownMenu.Item
              >
              <DropdownMenu.Item onSelect={() => (shown = block)}>{m.terminal_block_show_text()}</DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu.Root>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<BlockOutputDialog {address} block={shown} onClose={() => (shown = null)} />
