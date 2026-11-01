<script lang="ts">
  import { Button } from '@mannie/plugin-sdk/components/button';
  import * as Dialog from '@mannie/plugin-sdk/components/dialog';
  import { Spinner } from '@mannie/plugin-sdk/components/spinner';
  import { call } from '@mannie/plugin-sdk/ipc';

  import { type BlockTextView, type BlockView, commands } from '../generated/bindings.js';
  import { m } from '../generated/messages.js';
  import type { TerminalAddress, TerminalFailure } from '../types/terminal.js';
  import { terminalFailureMessage } from '../utils/terminal-messages.js';

  // A command's output as text (plugins/terminal/docs/README.md, the command
  // list), which a screen reader reads and the keyboard scrolls. Lines that
  // the terminal no longer has, or that a long output left out, show as `…`.
  interface Props {
    readonly address: TerminalAddress;
    /** The command whose output shows, or `null` while the dialog is closed. */
    readonly block: BlockView | null;
    readonly onClose: () => void;
  }

  let { address, block, onClose }: Props = $props();

  let text = $state<BlockTextView | null>(null);
  let failure = $state<TerminalFailure | null>(null);

  /** What the output holds, and how many of its lines are left out. */
  function description(): string {
    if (text === null) {
      return m.terminal_output_description();
    }
    return text.omitted === 0 ? m.terminal_output_description() : m.terminal_output_omitted({ count: text.omitted });
  }

  $effect(() => {
    const shown = block;
    text = null;
    failure = null;
    if (shown === null) {
      return;
    }
    let isCurrent = true;
    void call(() =>
      commands.blockText({ block: shown.number, conversationId: address.conversationId, terminal: address.terminal }),
    ).then((outcome) => {
      if (!isCurrent) {
        return;
      }
      if (outcome.ok) {
        text = outcome.data;
      } else {
        failure = outcome.failure;
      }
    });
    return () => {
      isCurrent = false;
    };
  });
</script>

<Dialog.Root
  bind:open={
    () => block !== null,
    (open) => {
      if (!open) {
        onClose();
      }
    }
  }
>
  <Dialog.Content class="sm:max-w-3xl">
    <Dialog.Header>
      <Dialog.Title class="font-mono break-all" data-content dir="ltr">{block?.command ?? ''}</Dialog.Title>
      <Dialog.Description>{description()}</Dialog.Description>
    </Dialog.Header>
    {#if failure !== null}
      <p class="text-sm text-destructive" role="alert">{terminalFailureMessage(failure)}</p>
    {:else if text === null}
      <div class="flex items-center gap-2 text-sm text-muted-foreground" role="status">
        <Spinner />
        {m.terminal_output_loading()}
      </div>
    {:else if text.lines.length === 0}
      <p class="text-sm text-muted-foreground">{m.terminal_output_empty()}</p>
    {:else}
      <!-- Focusable, so that a long output scrolls from the keyboard (WCAG 2.1.1). -->
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <pre
        class="max-h-[60vh] overflow-auto rounded-md bg-muted p-3 text-xs"
        aria-label={m.terminal_output_label()}
        data-content
        dir="ltr"
        tabindex="0">{text.lines.join('\n')}</pre>
    {/if}
    <Dialog.Footer>
      <Button variant="outline" onclick={onClose}>{m.terminal_close()}</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
